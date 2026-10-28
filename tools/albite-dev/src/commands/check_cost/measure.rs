//! One sample of `albite1 check src/compiler/main.tg`, and the run order (D1, D5).

use crate::commands::self_compile::telemetry::run_with_rusage;
use cost_baseline::Samples;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

/// The input every sample checks, relative to its tree's root.
pub const CHECK_INPUT: &str = "src/compiler/main.tg";

/// D1: `ulimit -s unlimited` on this process, before anything is spawned, so
/// every child inherits it. Kept by ADR 19.10.26c D5 although a current
/// `albite1` sizes its own stack: the A/B's base arm is built from a commit
/// that can predate that runtime entry, and such a `albite1` overflows the
/// container's 8 MB default.
///
/// # Errors
/// A hard limit that is not unlimited, or a failed `setrlimit`: the gate then
/// refuses rather than measure on 8 MB.
pub fn raise_stack_limit() -> Result<(), String> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit/setrlimit write and read only the struct passed.
    if unsafe { libc::getrlimit(libc::RLIMIT_STACK, &mut limit) } != 0 {
        return Err(format!(
            "getrlimit(RLIMIT_STACK): {}",
            std::io::Error::last_os_error()
        ));
    }
    if limit.rlim_max != libc::RLIM_INFINITY {
        return Err(format!(
            "the hard stack limit is {} — `check-cost` measures under an unlimited stack (D1)",
            limit.rlim_max
        ));
    }
    limit.rlim_cur = libc::RLIM_INFINITY;
    // SAFETY: as above.
    if unsafe { libc::setrlimit(libc::RLIMIT_STACK, &limit) } != 0 {
        return Err(format!(
            "setrlimit(RLIMIT_STACK): {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

/// One run: the wall time around the spawn and the reap, `ru_maxrss` from
/// `wait4`, and the exit code (-1 for a signal).
pub struct Sample {
    pub wall_s: f64,
    pub max_rss_kb: u64,
    pub exit_code: i32,
}

/// Run `binary` with `args` from `root`.
///
/// # Errors
/// A spawn failure.
pub fn sample(root: &Path, binary: &Path, args: &[&str]) -> Result<Sample, String> {
    let mut cmd = Command::new(binary);
    cmd.args(args).current_dir(root);
    let started = Instant::now();
    let out = run_with_rusage(&mut cmd).map_err(|e| format!("run {}: {e}", binary.display()))?;
    Ok(Sample {
        wall_s: started.elapsed().as_secs_f64(),
        max_rss_kb: out.max_rss_kb,
        exit_code: out.status.code().unwrap_or(-1),
    })
}

/// One step of an interleaved run: which arm, and whether it is discarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub arm: usize,
    pub warm_up: bool,
}

/// D5's order: one warm-up per arm, then the arms alternating, `runs` each.
pub fn run_order(runs: u32, arms: usize) -> Vec<Step> {
    let warm = (0..arms).map(|arm| Step { arm, warm_up: true });
    let measured = (0..runs).flat_map(|_| {
        (0..arms).map(|arm| Step {
            arm,
            warm_up: false,
        })
    });
    warm.chain(measured).collect()
}

/// Copy `tree`'s `src/` — tracked and untracked, ignored files (the `.tungsten`
/// caches) left out — to `dest/src`, replacing what was there, and return
/// `dest`: the root a candidate arm checks from, on the same filesystem as the
/// base worktree.
///
/// # Errors
/// A failing `git ls-files` or a copy that fails.
pub fn snapshot_src(tree: &Path, dest: &Path) -> Result<(), String> {
    let listing = super::ab::run_git(
        tree,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            "src",
        ],
    )?;
    let src = dest.join("src");
    if src.exists() {
        std::fs::remove_dir_all(&src).map_err(|e| format!("{}: {e}", src.display()))?;
    }
    for name in listing.lines() {
        let from = tree.join(name);
        if !from.is_file() {
            continue; // a tracked file this tree deleted
        }
        let to = dest.join(name);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::copy(&from, &to).map_err(|e| format!("copy {name}: {e}"))?;
    }
    Ok(())
}

/// A labelled arm: the tree whose `main.tg` it checks and the binary it runs.
pub struct Arm<'a> {
    pub root: &'a Path,
    pub binary: &'a Path,
}

/// Run every arm in [`run_order`], printing each sample, and return one
/// [`Samples`] per arm with the warm-ups discarded.
///
/// # Errors
/// A spawn failure.
pub fn run_arms(arms: &[Arm<'_>], runs: u32) -> Result<Vec<Samples>, String> {
    let mut out = vec![Samples::default(); arms.len()];
    for step in run_order(runs, arms.len()) {
        let arm = &arms[step.arm];
        let s = sample(arm.root, arm.binary, &["check", CHECK_INPUT])?;
        eprintln!(
            "  [check-cost] arm {} {}: {:.2} s, {} KiB, exit {}",
            step.arm,
            if step.warm_up { "warm-up" } else { "run" },
            s.wall_s,
            s.max_rss_kb,
            s.exit_code
        );
        if !step.warm_up {
            out[step.arm].wall_s.push(s.wall_s);
            out[step.arm].max_rss_kb.push(s.max_rss_kb);
            out[step.arm].exit_codes.push(s.exit_code);
        }
    }
    Ok(out)
}
