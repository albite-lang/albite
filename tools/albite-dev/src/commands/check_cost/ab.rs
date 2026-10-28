//! The A/B's base arm: a kept worktree at the window's base, built by the
//! base's OWN tools (D5), so a change to the runtime, the codegen or the link
//! flags is never shared by both arms and hidden.
//!
//! Everything lives under one container-local directory on the build volume
//! (P0.2): the worktree, its own `CARGO_TARGET_DIR`, the phase file and the
//! candidate's `src/` snapshot. Every arm reads its `main.tg` from the volume:
//! one `albite1` checking a bind-mounted `main.tg` ran 4.6% slower than the
//! same binary on a volume copy, and noisier under host load, so arms on
//! different filesystems are not a same-container A/B (ADR 15.10.26f §
//! Outcome). A kept worktree rebuilds only what the base moved; a fresh one
//! per run would rebuild everything cold.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::commands::self_compile::artifacts::SELF_COMPILE_OUTPUT;

/// The submodules a workspace build needs: a `git worktree` comes up without
/// them, and cargo refuses a workspace whose member manifests are missing.
const SUBMODULES: [&str; 4] = [
    "tools/claude-hooks",
    "tools/scs-notify",
    "tools/code-health",
    "tools/scs-snapshot",
];

/// The kept directory's layout.
pub struct Layout {
    pub root: PathBuf,
    pub worktree: PathBuf,
    pub target_dir: PathBuf,
    pub phase_file: PathBuf,
    /// Where the candidate's `src/` is copied before it is measured.
    pub candidate_snapshot: PathBuf,
}

impl Layout {
    /// Under `$CARGO_TARGET_DIR/check-cost/`: P0.2 found the volume's parent
    /// (`/build`) root-owned, and the volume persists across container
    /// recreates, which `/tmp` does not.
    pub fn under(cargo_target_dir: &Path) -> Layout {
        let root = cargo_target_dir.join("check-cost");
        Layout {
            worktree: root.join("base-tree"),
            target_dir: root.join("base-target"),
            phase_file: root.join("phase.toml"),
            candidate_snapshot: root.join("candidate-src"),
            root,
        }
    }

    /// The base arm's binary. The base's OWN albite-dev builds it, and one
    /// that predates ADR 16.10.26p writes `albite1` at the worktree root
    /// rather than under `out/`, so the newer of the two is the one built.
    pub fn base_binary(&self) -> PathBuf {
        let current = self.worktree.join(SELF_COMPILE_OUTPUT);
        let legacy = self.worktree.join("albite1");
        let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let (current_mtime, legacy_mtime) = (mtime(&current), mtime(&legacy));
        newer_of(current, current_mtime, legacy, legacy_mtime)
    }
}

/// The path with the later mtime; `current` on a tie or when neither exists.
pub fn newer_of<T: Ord>(
    current: PathBuf,
    current_mtime: Option<T>,
    legacy: PathBuf,
    legacy_mtime: Option<T>,
) -> PathBuf {
    if legacy_mtime > current_mtime {
        legacy
    } else {
        current
    }
}

/// Move the kept worktree to `base`, creating it on first use, and bring its
/// submodules up from the superproject's own module stores (no network).
///
/// # Errors
/// A failing `git` step.
pub fn checkout_base(repo: &Path, layout: &Layout, base: &str) -> Result<(), String> {
    if let Some(parent) = layout.worktree.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    if layout.worktree.join(".git").exists() {
        run_git(&layout.worktree, &["checkout", "--detach", "--quiet", base])?;
    } else {
        let _ = run_git(repo, &["worktree", "prune"]);
        let wt = layout.worktree.to_string_lossy().to_string();
        run_git(repo, &["worktree", "add", "--detach", &wt, base])?;
    }
    for name in SUBMODULES {
        let store = repo.join(".git/modules").join(name);
        let url = format!("submodule.{name}.url={}", store.display());
        run_git(
            &layout.worktree,
            &[
                "-c",
                "protocol.file.allow=always",
                "-c",
                &url,
                "submodule",
                "update",
                "--init",
                "--quiet",
                name,
            ],
        )?;
    }
    Ok(())
}

/// `base-tools`: the base's `albite-dev`, then its bootstrap, both into the
/// worktree's own target dir.
///
/// # Errors
/// A failing build.
pub fn build_base_tools(layout: &Layout) -> Result<(), String> {
    run_in_worktree(
        layout,
        Path::new("cargo"),
        &["build", "--release", "-p", "albite-dev"],
    )?;
    run_in_worktree(layout, &base_dev_tool(layout), &["ensure-bootstrap"])
}

/// `base-compile`: the base's own `self-compile --fast`, in the worktree.
///
/// # Errors
/// A failing self-compile.
pub fn compile_base(layout: &Layout) -> Result<(), String> {
    run_in_worktree(layout, &base_dev_tool(layout), &["self-compile", "--fast"])
}

fn base_dev_tool(layout: &Layout) -> PathBuf {
    layout.target_dir.join("release/albite-dev")
}

fn run_in_worktree(layout: &Layout, program: &Path, args: &[&str]) -> Result<(), String> {
    eprintln!(
        "  [check-cost] base arm: {} {}",
        program.display(),
        args.join(" ")
    );
    let status = Command::new(program)
        .args(args)
        .current_dir(&layout.worktree)
        .env("CARGO_TARGET_DIR", &layout.target_dir)
        .status()
        .map_err(|e| format!("{}: {e}", program.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "base arm `{} {}` exited {}",
            program.display(),
            args.join(" "),
            status.code().unwrap_or(-1)
        ))
    }
}

/// The bind-mounted workspace root's reported owner flips between `vscode`
/// and root under Docker Desktop's virtiofs, and git then refuses it as
/// "dubious ownership" mid-gate. `safe.directory` is honoured on the command
/// line, so the gate trusts the trees it reads without editing any config.
pub const SAFE_DIRECTORY: [&str; 2] = ["-c", "safe.directory=*"];

/// `git -C <dir> <args>`, its stdout trimmed.
///
/// # Errors
/// A spawn failure or a non-zero exit, with git's stderr.
pub fn run_git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(SAFE_DIRECTORY)
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}
