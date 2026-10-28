//! `check-cost bless` (D6): the operator's re-bless of `[measured]` and
//! `[environment]`. It never writes `[bound]`.

use super::ab::{run_git, Layout};
use super::measure::{self, Arm};
use super::phases::build_candidate;
use super::{environment, seed_bound, window_paths, Refusal};
use crate::commands::self_compile::artifacts::Artifacts;
use crate::commands::self_compile::stamp;
use cost_baseline::policy::applies;
use cost_baseline::record::{self, Measured, Record, RECORD_PATH};
use cost_baseline::verdict::spread_pct;
use cost_baseline::{Bound, Samples};
use std::process::Command;

pub(crate) fn bless() -> Result<i32, Refusal> {
    measure::raise_stack_limit()?;
    let arts = Artifacts::resolve()?;
    let head = run_git(&arts.root, &["rev-parse", "HEAD"])?;
    let head_record = run_git(&arts.root, &["show", &format!("HEAD:{RECORD_PATH}")]).ok();
    let in_batch = ["ACTION_BATCH_SINCE", "ACTION_BATCH_DEADLINE"]
        .iter()
        .any(|v| std::env::var(v).is_ok_and(|s| !s.is_empty()));
    let window = window_paths(&arts.root, "HEAD")?;
    if let Some(refusal) = bless_refusal(&window, in_batch, head_record.is_some()) {
        return Err(Refusal(refusal));
    }
    let bound = std::fs::read_to_string(arts.root.join(RECORD_PATH))
        .ok()
        .and_then(|t| record::parse(&t).ok())
        .map_or_else(seed_bound, |r| r.bound);
    let (stamp_head, digest) = stamp::current(&arts.root)?;
    let fresh = stamp::read_beside(&arts.output_binary)
        .is_some_and(|s| stamp::matches(&s, &stamp_head, digest));
    if !fresh {
        build_candidate(&arts.root, None)?;
    }
    // Measured from a volume copy, as every gate arm is (`ab::Layout`).
    let snapshot = Layout::under(&arts.target_dir).candidate_snapshot;
    measure::snapshot_src(&arts.root, &snapshot)?;
    let arms = [Arm {
        root: &snapshot,
        binary: &arts.output_binary,
    }];
    let samples = measure::run_arms(&arms, bound.runs)?.remove(0);
    if let Some(refusal) = samples_refusal(&samples, &bound) {
        return Err(Refusal(refusal));
    }
    let blessed = Record {
        bound,
        measured: Measured {
            commit: head,
            blessed_on: utc_date()?,
            wall_s: samples.wall_s,
            max_rss_kb: samples.max_rss_kb,
        },
        environment: environment::read_current(),
    };
    std::fs::write(arts.root.join(RECORD_PATH), record::render(&blessed)?)
        .map_err(|e| format!("write {RECORD_PATH}: {e}"))?;
    println!(
        "check-cost: blessed {RECORD_PATH} at {}",
        blessed.measured.commit
    );
    Ok(0)
}

/// D6's refusals before anything is measured: a trigger path other than the
/// record differing from HEAD (so `commit = HEAD` would be false of the binary
/// measured), or a batch re-blessing a record HEAD already has.
pub(crate) fn bless_refusal(
    window: &[String],
    in_batch: bool,
    head_has_record: bool,
) -> Option<String> {
    let dirty: Vec<&String> = window
        .iter()
        .filter(|p| p.as_str() != RECORD_PATH && applies(&[p.as_str()]))
        .collect();
    if let Some(first) = dirty.first() {
        return Some(format!(
            "{} path(s) under the trigger prefixes differ from HEAD ({first}, …), so `commit = HEAD` would be false (D6)",
            dirty.len()
        ));
    }
    (in_batch && head_has_record).then(|| {
        "a batch may only seed the record, and HEAD already has one — a re-bless is a human commit (D6)".into()
    })
}

/// D6: a failing or noisy bless writes nothing.
pub(crate) fn samples_refusal(samples: &Samples, bound: &Bound) -> Option<String> {
    if samples.any_failed() || samples.wall_s.is_empty() {
        return Some("a sample's check exited non-zero — not a cost (D1)".into());
    }
    let (wall, rss) = (spread_pct(&samples.wall_s), spread_pct(&samples.rss_f64()));
    (wall > bound.noise_pct || rss > bound.noise_pct).then(|| {
        format!(
            "the samples are noisy (spread {wall:.1}%/{rss:.1}% past {}%) — rerun on an idle container (D3)",
            bound.noise_pct
        )
    })
}

fn utc_date() -> Result<String, String> {
    let out = Command::new("date")
        .args(["-u", "+%F"])
        .output()
        .map_err(|e| format!("date: {e}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
