//! The freshness stamp `albite1.inputs` (ADR 15.10.26f D10).
//!
//! Every self-compile mode writes it beside `./albite1`, so `check-cost` can
//! tell whether the binary on disk was built from the tree it is about to
//! measure — and built the way it measures (`--fast`, no `--alloc-profile`,
//! not x86) — without a second ~5 min build. The digest is `std`'s
//! `DefaultHasher`: a local cache key, never committed, so it needs to be
//! stable only within one toolchain.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::process::Command;

/// The paths whose content the digest covers (D10).
pub const STAMPED_PATHS: [&str; 7] = [
    "src/",
    "bootstrap/",
    "albite_core/",
    "albite_codegen/",
    "albite_runtime/",
    "tools/albite-dev/src/commands/self_compile/",
    "Cargo.lock",
];

/// The stamp's file name, beside the output binary.
pub const STAMP_FILE: &str = "albite1.inputs";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    pub head: String,
    /// `fast`, `opt` or `direct`.
    pub mode: String,
    pub alloc_profile: bool,
    pub x86: bool,
    pub digest: u64,
}

impl Stamp {
    #[must_use]
    pub fn render(&self) -> String {
        format!(
            "head={}\nmode={}\nalloc_profile={}\nx86={}\ndigest={:016x}\n",
            self.head, self.mode, self.alloc_profile, self.x86, self.digest
        )
    }

    /// `None` for anything but a complete stamp: a partial one is stale.
    #[must_use]
    pub fn parse(text: &str) -> Option<Stamp> {
        let field = |key: &str| {
            text.lines()
                .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
                .map(str::to_string)
        };
        Some(Stamp {
            head: field("head")?,
            mode: field("mode")?,
            alloc_profile: field("alloc_profile")?.parse().ok()?,
            x86: field("x86")?.parse().ok()?,
            digest: u64::from_str_radix(&field("digest")?, 16).ok()?,
        })
    }
}

/// `tree=<head, 12 hex>-<digest>`: what `/check-adr` compares to decide whether
/// a recorded verdict line still describes the tree (D11).
#[must_use]
pub fn tree_label(head: &str, digest: u64) -> String {
    let head: String = head.chars().take(12).collect();
    format!("{head}-{digest:016x}")
}

/// D10: the binary is the measurable one for this tree — built `--fast`, with
/// no allocation profiling, for arm64, from exactly these inputs.
#[must_use]
pub fn matches(recorded: &Stamp, current_head: &str, current_digest: u64) -> bool {
    recorded.mode == "fast"
        && !recorded.alloc_profile
        && !recorded.x86
        && recorded.head == current_head
        && recorded.digest == current_digest
}

/// The digest over `git diff HEAD --binary` and the untracked files (name and
/// content), both over [`STAMPED_PATHS`].
#[must_use]
pub fn digest(diff: &[u8], untracked: &[(String, Vec<u8>)]) -> u64 {
    let mut hasher = DefaultHasher::new();
    diff.hash(&mut hasher);
    for (name, content) in untracked {
        name.hash(&mut hasher);
        content.hash(&mut hasher);
    }
    hasher.finish()
}

/// Read `HEAD` and the digest for the tree at `root`.
///
/// # Errors
/// A `git` invocation that fails.
pub fn current(root: &Path) -> Result<(String, u64), String> {
    let head = git(root, &["rev-parse", "HEAD"])?;
    let head = String::from_utf8_lossy(&head).trim().to_string();
    let mut diff_args = vec!["diff", "HEAD", "--binary", "--"];
    diff_args.extend(STAMPED_PATHS);
    let diff = git(root, &diff_args)?;
    let mut list_args = vec!["ls-files", "--others", "--exclude-standard", "-z", "--"];
    list_args.extend(STAMPED_PATHS);
    let listing = git(root, &list_args)?;
    let mut untracked = Vec::new();
    for name in listing.split(|&b| b == 0).filter(|n| !n.is_empty()) {
        let name = String::from_utf8_lossy(name).to_string();
        let content = std::fs::read(root.join(&name)).unwrap_or_default();
        untracked.push((name, content));
    }
    Ok((head, digest(&diff, &untracked)))
}

/// Write the stamp for a build just finished at `root`. A failure warns: the
/// stamp is a cache key, and a missing one costs only a rebuild.
pub fn write_after_build(root: &Path, output_binary: &Path, mode: &str, flags: (bool, bool)) {
    let (alloc_profile, x86) = flags;
    let stamp_path = output_binary.with_file_name(STAMP_FILE);
    let written = current(root).and_then(|(head, digest)| {
        let stamp = Stamp {
            head,
            mode: mode.to_string(),
            alloc_profile,
            x86,
            digest,
        };
        std::fs::write(&stamp_path, stamp.render()).map_err(|e| e.to_string())
    });
    if let Err(e) = written {
        eprintln!(
            "⚠ freshness stamp not written ({}): {e}",
            stamp_path.display()
        );
    }
}

/// Read the stamp beside `output_binary`, if there is a complete one.
#[must_use]
pub fn read_beside(output_binary: &Path) -> Option<Stamp> {
    let text = std::fs::read_to_string(output_binary.with_file_name(STAMP_FILE)).ok()?;
    Stamp::parse(&text)
}

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    // The bind mount's reported owner can flip to root mid-session; trust the
    // tree on the command line rather than in any config (as `check_cost::ab`).
    let out = Command::new("git")
        .args(["-c", "safe.directory=*"])
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast(head: &str, digest: u64) -> Stamp {
        Stamp {
            head: head.into(),
            mode: "fast".into(),
            alloc_profile: false,
            x86: false,
            digest,
        }
    }

    #[test]
    fn stamp_round_trips_and_a_partial_one_is_none() {
        let stamp = fast("abc123", 0xdead_beef);
        assert_eq!(Stamp::parse(&stamp.render()), Some(stamp.clone()));
        assert_eq!(Stamp::parse("head=abc\nmode=fast\n"), None);
        assert_eq!(
            tree_label(&stamp.head, stamp.digest),
            "abc123-00000000deadbeef"
        );
        assert_eq!(
            tree_label("0123456789abcdef", 1),
            "0123456789ab-0000000000000001"
        );
    }

    /// 15.10.26f AC4: `matches` is false when HEAD moves, when a tracked or an
    /// untracked input changes the digest, and for any mode or flag the gate
    /// does not measure; true otherwise.
    #[test]
    fn matches_only_the_fast_build_of_this_tree() {
        let d = digest(b"diff", &[]);
        let recorded = fast("abc", d);
        assert!(matches(&recorded, "abc", d));
        assert!(!matches(&recorded, "abd", d), "HEAD moved");

        let tracked_change = digest(b"diff2", &[]);
        assert_ne!(tracked_change, d);
        assert!(
            !matches(&recorded, "abc", tracked_change),
            "a tracked file changed"
        );

        let untracked = digest(b"diff", &[("src/compiler/new.tg".into(), b"x".to_vec())]);
        assert_ne!(untracked, d);
        assert!(
            !matches(&recorded, "abc", untracked),
            "an untracked file appeared"
        );

        for mode in ["opt", "direct"] {
            let mut other = recorded.clone();
            other.mode = mode.into();
            assert!(!matches(&other, "abc", d), "{mode}");
        }
        let mut alloc = recorded.clone();
        alloc.alloc_profile = true;
        assert!(!matches(&alloc, "abc", d), "alloc_profile");
        let mut x86 = recorded.clone();
        x86.x86 = true;
        assert!(!matches(&x86, "abc", d), "x86");
    }

    #[test]
    fn digest_covers_untracked_names_and_contents() {
        let a = digest(b"", &[("a".into(), b"1".to_vec())]);
        assert_ne!(a, digest(b"", &[("b".into(), b"1".to_vec())]));
        assert_ne!(a, digest(b"", &[("a".into(), b"2".to_vec())]));
        assert_eq!(a, digest(b"", &[("a".into(), b"1".to_vec())]));
    }

    /// 15.10.26f AC4: the digest of a real tree moves with a tracked edit and
    /// with a new untracked file under a stamped path, and not outside one.
    #[test]
    fn current_tracks_a_real_tree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let sh = |cmd: &str| {
            let ok = Command::new("sh")
                .args(["-c", cmd])
                .current_dir(root)
                .status()
                .unwrap();
            assert!(ok.success(), "{cmd}");
        };
        sh("git init -q && git config user.email t@t && git config user.name t");
        sh("mkdir -p src && echo a > src/a.tg && echo n > notes.md && git add -A && git commit -qm i");
        let (head, clean) = current(root).unwrap();
        assert_eq!(head.len(), 40);
        sh("echo n2 > notes.md");
        assert_eq!(current(root).unwrap().1, clean, "outside the stamped paths");
        sh("echo b > src/a.tg");
        let edited = current(root).unwrap().1;
        assert_ne!(edited, clean, "tracked edit");
        sh("echo c > src/new.tg");
        assert_ne!(current(root).unwrap().1, edited, "untracked file");

        let binary = root.join("albite1");
        write_after_build(root, &binary, "fast", (false, false));
        let stamp = read_beside(&binary).unwrap();
        let (head_now, digest_now) = current(root).unwrap();
        assert!(matches(&stamp, &head_now, digest_now));
    }
}
