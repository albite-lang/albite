//! The five phases (D7). Each returns where the plan now stands; a decided
//! verdict has already printed its line.

use super::context::Context;
use super::measure::{self, Arm};
use super::{ab, GateArgs, Refusal};
use crate::commands::self_compile::stamp;
use cost_baseline::phase::{Phase, PhaseState};
use cost_baseline::verdict::{
    classify_stored, compare, plan, resolve_ab, Basis, Comparison, Next, PlanInputs, StoredClass,
};
use std::path::Path;
use std::process::Command;

pub(crate) fn run_phase(
    ctx: &Context,
    state: &mut PhaseState,
    phase: Phase,
    args: &GateArgs,
) -> Result<Next, Refusal> {
    if phase != Phase::Prepare && state.inputs_moved(&ctx.base, &ctx.tree) {
        return Err(Refusal(format!(
            "phase {} over stale phase state: the base or the tree moved since prepare — rerun from --phase prepare (D7)",
            phase.name()
        )));
    }
    match phase {
        Phase::Prepare => prepare(ctx, state, args),
        Phase::Measure => measure_stored(ctx, state, args),
        Phase::BaseTools => {
            ab::checkout_base(&ctx.repo, &ctx.layout, &ctx.base)?;
            ab::build_base_tools(&ctx.layout)?;
            Ok(Next::RunAb)
        }
        Phase::BaseCompile => compile_base_once(ctx, state),
        Phase::Ab => run_ab(ctx, state),
    }
}

fn plan_inputs<'a>(
    ctx: &'a Context,
    args: &GateArgs,
    stored: Option<StoredClass>,
) -> PlanInputs<'a> {
    PlanInputs {
        applies: ctx.applies,
        record: &ctx.record,
        stale: &ctx.stale,
        stored,
        no_ab: args.no_ab,
        force_ab: args.force_ab,
    }
}

fn prepare(ctx: &Context, state: &mut PhaseState, args: &GateArgs) -> Result<Next, Refusal> {
    *state = state.restart(&ctx.base, &ctx.tree);
    let next = plan(&plan_inputs(ctx, args, None));
    if let Next::Decided(verdict) = next {
        println!("{}", ctx.line(verdict, None).render());
        return Ok(next);
    }
    if args.rebuild || !ctx.candidate_fresh()? {
        build_candidate(&ctx.candidate, ctx.candidate_target.as_deref())?;
    }
    let arms = if next == Next::RunAb {
        "the kept base worktree and the candidate (A/B)"
    } else {
        "the candidate against the stored record"
    };
    eprintln!(
        "  [check-cost] plan: {arms}; base {} tree {}",
        ctx.base, ctx.tree
    );
    Ok(next)
}

/// D10: no flag skips a stale build — `self-compile --fast` first.
pub(crate) fn build_candidate(root: &Path, target: Option<&Path>) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let mut cmd = Command::new(exe);
    cmd.args(["self-compile", "--fast"]).current_dir(root);
    if let Some(dir) = target {
        cmd.env("CARGO_TARGET_DIR", dir);
    }
    let status = cmd.status().map_err(|e| format!("self-compile: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "candidate self-compile --fast exited {}",
            status.code().unwrap_or(-1)
        ))
    }
}

fn require_fresh(ctx: &Context) -> Result<(), Refusal> {
    if ctx.candidate_fresh()? {
        Ok(())
    } else {
        Err(Refusal(format!(
            "{} is stale for this tree ({}) — rerun from --phase prepare (D10)",
            ctx.candidate_binary().display(),
            stamp::STAMP_FILE
        )))
    }
}

fn measure_stored(ctx: &Context, state: &mut PhaseState, args: &GateArgs) -> Result<Next, Refusal> {
    let Some(record) = ctx.present() else {
        return Err(Refusal(
            "measure needs a record at the base; the plan goes to the A/B (D2)".into(),
        ));
    };
    require_fresh(ctx)?;
    measure::snapshot_src(&ctx.candidate, &ctx.layout.candidate_snapshot)?;
    let binary = ctx.candidate_binary();
    let arms = [Arm {
        root: &ctx.layout.candidate_snapshot,
        binary: &binary,
    }];
    let candidate = measure::run_arms(&arms, record.bound.runs)?.remove(0);
    state.record_stored(&candidate);
    let stored = compare(&record.measured.samples(), &candidate, &record.bound);
    let next = plan(&plan_inputs(
        ctx,
        args,
        Some(classify_stored(&stored, &record.bound)),
    ));
    if let Next::Decided(verdict) = next {
        let mut line = ctx.line(verdict, Some(ctx.stored_basis()));
        line.decided_by = Some(stored);
        println!("{}", line.render());
    }
    Ok(next)
}

fn compile_base_once(ctx: &Context, state: &mut PhaseState) -> Result<Next, Refusal> {
    let at = ab::run_git(&ctx.layout.worktree, &["rev-parse", "HEAD"])?;
    if at != ctx.base {
        return Err(Refusal(format!(
            "the base worktree is at {at}, not {} — run --phase base-tools first (D7)",
            ctx.base
        )));
    }
    if state.ab_built_base == ctx.base && ctx.layout.base_binary().exists() {
        eprintln!(
            "  [check-cost] base arm: ./albite1 already built from {}",
            ctx.base
        );
    } else {
        ab::compile_base(&ctx.layout)?;
        state.ab_built_base.clone_from(&ctx.base);
    }
    Ok(Next::RunAb)
}

fn run_ab(ctx: &Context, state: &PhaseState) -> Result<Next, Refusal> {
    if state.ab_built_base != ctx.base {
        return Err(Refusal(
            "the base arm is not built for this base — run --phase base-compile first (D7)".into(),
        ));
    }
    require_fresh(ctx)?;
    measure::snapshot_src(&ctx.candidate, &ctx.layout.candidate_snapshot)?;
    let bound = ctx.bound();
    let (base_bin, cand_bin) = (ctx.layout.base_binary(), ctx.candidate_binary());
    let arms = [
        Arm {
            root: &ctx.layout.worktree,
            binary: &base_bin,
        },
        Arm {
            root: &ctx.layout.candidate_snapshot,
            binary: &cand_bin,
        },
    ];
    let mut samples = measure::run_arms(&arms, bound.runs)?;
    let (candidate, base) = (samples.remove(1), samples.remove(0));
    let ab_cmp = compare(&base, &candidate, &bound);
    let stored = stored_comparison(ctx, state);
    let verdict = resolve_ab(stored.map(|(_, class)| class), &ab_cmp);
    let mut line = ctx.line(verdict, Some(Basis::Ab));
    line.decided_by = Some(ab_cmp);
    line.stored = stored.map(|(c, _)| c);
    println!("{}", line.render());
    Ok(Next::Decided(verdict))
}

/// The stored comparison `measure` ran, recomputed from the phase file — but
/// only over a fresh record: a stale one's numbers are meaningless (D5).
fn stored_comparison(ctx: &Context, state: &PhaseState) -> Option<(Comparison, StoredClass)> {
    let record = ctx.present().filter(|_| ctx.stale.is_empty())?;
    let candidate = state.stored_samples()?;
    let c = compare(&record.measured.samples(), &candidate, &record.bound);
    Some((c, classify_stored(&c, &record.bound)))
}
