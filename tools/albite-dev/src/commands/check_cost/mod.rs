//! `albite-dev check-cost` — `albite1 check main.tg` against a stored cost
//! (ADR 15.10.26f).
//!
//! Every decision is `cost_baseline`'s, a pure crate the diff gates score;
//! this module only runs things: `git`, `setrlimit`, the samples, the base
//! arm's builds and the phase file. It prints one `check-cost:` line per
//! phase — the verdict, or the next command — and the line, not the exit, is
//! what a child records, because make collapses every failing exit to 2.

pub mod ab;
mod bless;
mod context;
pub mod environment;
pub mod measure;
mod phases;
#[cfg(test)]
mod tests;

use ab::{run_git, Layout};
use clap::{Args, Subcommand};
use context::Context;
use cost_baseline::phase::{deadline_fits, next_command, next_phase, Phase, PhaseState};
use cost_baseline::verdict::Next;
use cost_baseline::{Bound, Verdict};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Args)]
pub struct CheckCostArgs {
    #[command(subcommand)]
    pub action: CheckCostAction,
}

#[derive(Subcommand)]
pub enum CheckCostAction {
    /// Compare `albite1 check src/compiler/main.tg` with the stored record,
    /// and A/B it against the window's base when the record cannot settle it
    Gate(GateArgs),
    /// Measure the working tree's `./albite1` and rewrite the record's
    /// `[measured]` and `[environment]` (never `[bound]`) — the operator's
    Bless,
}

#[derive(Args, Clone, Default)]
pub struct GateArgs {
    /// Run one phase: prepare, measure, base-tools, base-compile or ab.
    /// Omitted, every needed phase runs in one call (an operator's terminal only)
    #[arg(long)]
    pub phase: Option<String>,
    /// The window's base; default `ACTION_BATCH_SINCE`, else `HEAD`. The last
    /// one given wins, so `COST_ARGS` may override the make target's `BASE=`
    #[arg(long, overrides_with = "base")]
    pub base: Option<String>,
    /// Read the record from this file instead of the base (`basis=file`)
    #[arg(long)]
    pub baseline_file: Option<PathBuf>,
    /// Stop after the stored comparison (needs --baseline-file)
    #[arg(long)]
    pub no_ab: bool,
    /// Run the A/B even after a stored pass (the A/A against HEAD)
    #[arg(long)]
    pub force_ab: bool,
    /// Rebuild the candidate `./albite1` even when its stamp matches
    #[arg(long)]
    pub rebuild: bool,
    /// The candidate tree's root (default: the workspace)
    #[arg(long)]
    pub tree: Option<PathBuf>,
}

impl GateArgs {
    /// Every flag but `--phase` and `--rebuild`, for the next phase's command.
    pub(crate) fn carried(&self) -> String {
        let mut words = Vec::new();
        if let Some(base) = &self.base {
            words.push(format!("--base {base}"));
        }
        if let Some(file) = &self.baseline_file {
            words.push(format!("--baseline-file {}", file.display()));
        }
        if let Some(tree) = &self.tree {
            words.push(format!("--tree {}", tree.display()));
        }
        for (on, flag) in [(self.no_ab, "--no-ab"), (self.force_ab, "--force-ab")] {
            if on {
                words.push(flag.to_string());
            }
        }
        words.join(" ")
    }
}

/// D4's refusal: exit 2, the child's to undo.
pub(crate) struct Refusal(pub(crate) String);

impl From<String> for Refusal {
    fn from(message: String) -> Self {
        Refusal(message)
    }
}

pub fn run(args: CheckCostArgs) -> Result<(), String> {
    let outcome = match args.action {
        CheckCostAction::Gate(gate_args) => gate(&gate_args),
        CheckCostAction::Bless => bless::bless(),
    };
    match outcome {
        Ok(0) => Ok(()),
        Ok(code) => std::process::exit(code),
        Err(Refusal(reason)) => {
            println!("check-cost: refused: {reason}");
            std::process::exit(2)
        }
    }
}

/// D2's `[bound]`, used for the seeding and when the base has no record.
pub(crate) fn seed_bound() -> Bound {
    Bound {
        wall_pct: 10.0,
        rss_pct: 10.0,
        noise_pct: 8.0,
        drift_ceiling_pct: 20.0,
        stored_wall_margin_pct: 0.0,
        runs: 5,
        adr: "15.10.26f".into(),
    }
}

/// D9: `git diff --name-only <base>` plus the untracked files.
pub(crate) fn window_paths(root: &Path, base: &str) -> Result<Vec<String>, String> {
    let diff = run_git(root, &["diff", "--name-only", base])?;
    let untracked = run_git(root, &["ls-files", "--others", "--exclude-standard"])?;
    Ok(diff
        .lines()
        .chain(untracked.lines())
        .map(str::to_string)
        .collect())
}

fn gate(args: &GateArgs) -> Result<i32, Refusal> {
    if args.no_ab && args.baseline_file.is_none() {
        return Err(Refusal("--no-ab needs --baseline-file (D2)".into()));
    }
    measure::raise_stack_limit()?;
    let requested = match &args.phase {
        Some(name) => {
            Some(Phase::parse(name).ok_or_else(|| Refusal(format!("unknown phase `{name}`")))?)
        }
        None => None,
    };
    let mut phase = requested.unwrap_or(Phase::Prepare);
    loop {
        let ctx = Context::load(args)?;
        let mut state = PhaseState::parse(
            std::fs::read_to_string(&ctx.layout.phase_file)
                .ok()
                .as_deref(),
        );
        if !deadline_ok(&ctx, &state, phase) {
            return Ok(Verdict::Deadline.exit_code());
        }
        let started = Instant::now();
        let next = phases::run_phase(&ctx, &mut state, phase, args)?;
        state.record_duration(phase, started.elapsed().as_secs());
        save_state(&ctx.layout, &state)?;
        if let Next::Decided(verdict) = next {
            return Ok(verdict.exit_code());
        }
        let Some(following) = next_phase(phase, next) else {
            return Ok(0);
        };
        if requested.is_some() {
            println!(
                "check-cost: next=\"{}\"",
                next_command(following, &args.carried())
            );
            return Ok(0);
        }
        phase = following;
    }
}

/// D7: print the estimate, and refuse to start what the deadline cannot fit.
fn deadline_ok(ctx: &Context, state: &PhaseState, phase: Phase) -> bool {
    let estimate = state.estimate_s(phase);
    let deadline = std::env::var("ACTION_BATCH_DEADLINE")
        .ok()
        .and_then(|v| v.parse().ok());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    eprintln!(
        "  [check-cost] phase {}: estimate ~{estimate}s",
        phase.name()
    );
    if deadline_fits(estimate, deadline, now) {
        return true;
    }
    let left = deadline.map_or(0, |d: u64| d.saturating_sub(now));
    println!(
        "{} phase={} estimate={estimate}s left={left}s",
        ctx.line(Verdict::Deadline, None).render(),
        phase.name()
    );
    false
}

fn save_state(layout: &Layout, state: &PhaseState) -> Result<(), Refusal> {
    std::fs::create_dir_all(&layout.root).map_err(|e| format!("{}: {e}", layout.root.display()))?;
    std::fs::write(&layout.phase_file, state.render())
        .map_err(|e| Refusal(format!("{}: {e}", layout.phase_file.display())))
}
