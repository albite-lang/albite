//! What every phase reads, recomputed each call: phases run in separate
//! processes, and a phase whose inputs moved must notice (D7).

use super::ab::{run_git, Layout};
use super::{environment, seed_bound, window_paths, GateArgs, Refusal};
use crate::commands::self_compile::artifacts::{Artifacts, SELF_COMPILE_OUTPUT};
use crate::commands::self_compile::stamp;
use cost_baseline::policy::{applies, bound_change, stale_fields, BoundChange, Field};
use cost_baseline::record::{self, Record, RecordState, RECORD_PATH};
use cost_baseline::verdict::{Basis, VerdictLine};
use cost_baseline::{Bound, Verdict};
use std::path::{Path, PathBuf};

pub(crate) struct Context {
    pub candidate: PathBuf,
    /// The candidate's own target dir when it is not the workspace (`--tree`).
    pub candidate_target: Option<PathBuf>,
    pub layout: Layout,
    pub repo: PathBuf,
    pub base: String,
    pub tree: String,
    pub applies: bool,
    pub record: RecordState,
    pub stale: Vec<Field>,
    pub basis_file: bool,
}

impl Context {
    pub fn load(args: &GateArgs) -> Result<Context, Refusal> {
        let arts = Artifacts::resolve()?;
        let candidate = args.tree.clone().unwrap_or_else(|| arts.root.clone());
        let layout = Layout::under(&arts.target_dir);
        let base_rev = args
            .base
            .clone()
            .or_else(|| {
                std::env::var("ACTION_BATCH_SINCE")
                    .ok()
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_else(|| "HEAD".into());
        let base = run_git(
            &arts.root,
            &["rev-parse", &format!("{base_rev}^{{commit}}")],
        )?;
        let window = window_paths(&candidate, &base)?;
        let window_refs: Vec<&str> = window.iter().map(String::as_str).collect();
        let (record, basis_file) = read_record(&arts.root, &base, args.baseline_file.as_deref());
        if window_refs.contains(&RECORD_PATH) {
            refuse_widened_bound(&record, &candidate)?;
        }
        let stale = match &record {
            RecordState::Present(r) => stale_fields(&r.environment, &environment::read_current()),
            _ => Vec::new(),
        };
        let (head, digest) = stamp::current(&candidate)?;
        let candidate_target = (candidate != arts.root).then(|| layout.root.join("tree-target"));
        Ok(Context {
            applies: applies(&window_refs),
            tree: stamp::tree_label(&head, digest),
            candidate,
            candidate_target,
            layout,
            repo: arts.root,
            base,
            record,
            stale,
            basis_file,
        })
    }

    pub fn line(&self, verdict: Verdict, basis: Option<Basis>) -> VerdictLine {
        VerdictLine {
            verdict,
            basis,
            decided_by: None,
            stored: None,
            record: self.record.label(),
            stale: self.stale.clone(),
            base: self.base.clone(),
            tree: self.tree.clone(),
        }
    }

    pub fn stored_basis(&self) -> Basis {
        if self.basis_file {
            Basis::File
        } else {
            Basis::Stored
        }
    }

    pub fn present(&self) -> Option<&Record> {
        match &self.record {
            RecordState::Present(r) => Some(r),
            _ => None,
        }
    }

    /// The bound in force: the record's, else D2's seed (an absent record
    /// still needs a bound for the A/B).
    pub fn bound(&self) -> Bound {
        self.present().map_or_else(seed_bound, |r| r.bound.clone())
    }

    pub fn candidate_binary(&self) -> PathBuf {
        self.candidate.join(SELF_COMPILE_OUTPUT)
    }

    pub fn candidate_fresh(&self) -> Result<bool, String> {
        let (head, digest) = stamp::current(&self.candidate)?;
        Ok(stamp::read_beside(&self.candidate_binary())
            .is_some_and(|s| stamp::matches(&s, &head, digest)))
    }
}

/// D2: the record as it stands at the base — never the tree's copy, so a child
/// that edits it cannot change its own verdict — or `--baseline-file`.
fn read_record(repo: &Path, base: &str, file: Option<&Path>) -> (RecordState, bool) {
    match file {
        Some(path) => (
            RecordState::from_text(std::fs::read_to_string(path).ok().as_deref()),
            true,
        ),
        None => {
            let shown = run_git(repo, &["show", &format!("{base}:{RECORD_PATH}")]).ok();
            (RecordState::from_text(shown.as_deref()), false)
        }
    }
}

/// D6: a tree bound looser than the base's, with `adr` unchanged, refuses.
fn refuse_widened_bound(base: &RecordState, candidate: &Path) -> Result<(), Refusal> {
    let RecordState::Present(base) = base else {
        return Ok(());
    };
    let Ok(text) = std::fs::read_to_string(candidate.join(RECORD_PATH)) else {
        return Ok(());
    };
    let Ok(tree) = record::parse(&text) else {
        return Ok(());
    };
    match bound_change(&base.bound, &tree.bound) {
        BoundChange::Widened(fields) => Err(Refusal(format!(
            "the tree widens [bound] ({}) with adr unchanged — revert it; only an ADR widens the bound (D6)",
            fields.join(", ")
        ))),
        _ => Ok(()),
    }
}
