//! `--proof-bearing` (ADR 3.10.26s): AC 1's selection over injected trees and
//! parse outcomes, then AC 2 and AC 3 over tempdir corpora.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::corpus_tests::categories;
use super::{corpus, needs_delta_row, register, write};
use crate::doctor::checks::check_kernel_census::corpus::{census_corpus, render_corpus};
use crate::doctor::checks::check_kernel_census::entry_plan::{
    parse_for_proofs, parse_tree_files, plan_entries, select_proof_entries, walk_roots, EntryPlan,
    FileParse, ProofFacts,
};
use crate::doctor::checks::check_kernel_census::register_diff::{DiffCategory, NotExaminedWhy};
use crate::doctor::checks::check_kernel_census::FileOutcome;
use crate::elaborate::kernel::Register;

fn path(name: &str) -> PathBuf {
    PathBuf::from(format!("/corpus/{name}.tg"))
}

/// A plan of `entries`, each `(entry, tree files after itself)`, in order.
fn plan_of(entries: &[(&str, &[&str])]) -> EntryPlan {
    let mut plan = EntryPlan::default();
    for (entry, rest) in entries {
        plan.entries.push(path(entry));
        let tree = std::iter::once(*entry).chain(rest.iter().copied());
        plan.trees.insert(path(entry), tree.map(path).collect());
    }
    plan
}

fn kept_names(plan: &EntryPlan, parses: &BTreeMap<PathBuf, FileParse>) -> Vec<String> {
    let kept = select_proof_entries(plan, parses);
    plan.entries
        .iter()
        .filter(|e| kept.contains(*e))
        .map(|e| {
            e.file_stem()
                .expect("a stem")
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

/// 3.10.26s AC1: the first entry to reach a proof file is kept, a later one
/// sharing it is skipped unless it reaches another, and a proof-free tree is
/// skipped.
#[test]
fn the_selection_keeps_one_entry_per_uncovered_proof_file() {
    let plan = plan_of(&[
        ("first", &["shared", "proof"]),
        ("second", &["shared", "proof"]),
        ("plain", &["shared"]),
        ("wider", &["proof", "other_proof"]),
    ]);
    let mut parses: BTreeMap<PathBuf, FileParse> = ["first", "second", "plain", "wider", "shared"]
        .into_iter()
        .map(|name| (path(name), FileParse::ProofFree))
        .collect();
    parses.insert(path("proof"), FileParse::ProofBearing);
    parses.insert(path("other_proof"), FileParse::ProofBearing);
    assert_eq!(kept_names(&plan, &parses), ["first", "wider"]);
    // The entry file itself counts: a proof in the root keeps it.
    parses.insert(path("plain"), FileParse::ProofBearing);
    assert_eq!(kept_names(&plan, &parses), ["first", "plain", "wider"]);
}

/// 3.10.26s AC1: a child that failed to parse, a file never parsed, and an
/// unbuilt tree (an unresolved `mod`) all keep their entry.
#[test]
fn the_selection_fails_open() {
    let plan = plan_of(&[
        ("broken", &["bad"]),
        ("unparsed", &["unknown"]),
        ("unbuilt", &[]),
    ]);
    let mut parses: BTreeMap<PathBuf, FileParse> = ["broken", "unparsed", "unbuilt"]
        .into_iter()
        .map(|name| (path(name), FileParse::ProofFree))
        .collect();
    parses.insert(path("bad"), FileParse::Failed);
    assert_eq!(kept_names(&plan, &parses), ["broken", "unparsed"]);
    let mut unbuilt = plan;
    unbuilt.unbuilt.insert(path("unbuilt"));
    assert_eq!(
        kept_names(&unbuilt, &parses),
        ["broken", "unparsed", "unbuilt"]
    );
    // An entry with no tree row at all is kept too.
    unbuilt.unbuilt.clear();
    unbuilt.trees.remove(&path("unbuilt"));
    assert_eq!(
        kept_names(&unbuilt, &parses),
        ["broken", "unparsed", "unbuilt"]
    );
}

/// 3.10.26s AC1: what a parse says of a theorem, a lemma, an axiom, a
/// proof-free file, a syntax error and a missing file.
#[test]
fn a_parse_names_proof_bearing_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cases = [
        (
            "t.tg",
            "theorem t(): 1 == 1 { refl }\n",
            FileParse::ProofBearing,
        ),
        (
            "l.tg",
            "lemma l(): 1 == 1 { refl }\n",
            FileParse::ProofBearing,
        ),
        (
            "a.tg",
            "axiom trusted(n: Nat): Bool\n",
            FileParse::ProofBearing,
        ),
        ("f.tg", "fn main() -> Nat { 0 }\n", FileParse::ProofFree),
        (
            "e.tg",
            "fn main( -> Nat { 0 }\ntheorem t(): 1 == 1 { refl }\n",
            FileParse::Failed,
        ),
    ];
    for (name, source, expected) in cases {
        assert_eq!(verdict_of(&[(name, source)]), expected, "{name}");
    }
    assert_eq!(parse_for_proofs(&dir.path().join("missing.tg")), None);
}

/// The verdict on the first of `files`, all of them one entry's tree.
fn verdict_of(files: &[(&str, &str)]) -> FileParse {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths: Vec<PathBuf> = files
        .iter()
        .map(|(name, source)| {
            write(dir.path(), name, source);
            dir.path().join(name)
        })
        .collect();
    let mut plan = EntryPlan::default();
    plan.entries.push(paths[0].clone());
    plan.trees.insert(paths[0].clone(), paths.clone());
    parse_tree_files(&plan, parse_for_proofs)[&paths[0]]
}

/// 14.10.26b AC 1: a non-`#[partial]` `fn` whose return type mentions a
/// proposition makes its file proof-bearing, wherever the proposition sits.
#[test]
fn a_proof_fn_makes_its_file_proof_bearing() {
    let bearing = [
        "fn f(n: Nat) -> Eq<Nat, n, n> { refl }\n",
        "fn g() -> 0 == 0 { refl }\n",
        "fn h() -> (Eq<Nat, 0, 0>, Nat) { (refl, 0) }\n",
        "fn v() -> Void { sorry }\n",
        "fn o() -> Option<Eq<Nat, 0, 0>> { sorry }\n",
        "fn s() -> Eq<Nat, 0, 0> + Nat { sorry }\n",
        "fn k() -> Nat -> Eq<Nat, 0, 0> { sorry }\n",
        "fn d() -> (n: Nat) -> Eq<Nat, n, n> { sorry }\n",
        "fn e() -> exists n: Nat. n == n { sorry }\n",
        "fn q() -> (Eq<Nat, 0, 0>) { sorry }\n",
        "fn r() -> Ref<Eq<Nat, 0, 0>> { sorry }\n",
        "fn t() -> *Eq<Nat, 0, 0> { sorry }\n",
        "type R = { p: Eq<Nat, 0, 0>, n: Nat }\nfn r() -> R { sorry }\n",
        "type W = | Wrap(Nat, Eq<Nat, 0, 0>)\nfn w() -> W { sorry }\n",
        "type Holds(n: Nat): Prop = Opaque\nfn p() -> Holds<0> { sorry }\n",
    ];
    for source in bearing {
        assert_eq!(
            verdict_of(&[("f.tg", source)]),
            FileParse::ProofBearing,
            "{source}"
        );
    }
}

/// 14.10.26b AC 1: a `#[partial]` proof `fn`, a data `fn`, a `fn` that only
/// takes a proof, and a type that carries one but no `fn` returns.
#[test]
fn a_fn_that_returns_no_proof_leaves_its_file_proof_free() {
    let free = [
        "#[partial]\nfn spin(n: Nat) -> Eq<Nat, n, n> { spin(n) }\n",
        "fn d() -> Nat { 0 }\n",
        "fn takes(p: Eq<Nat, 0, 0>) -> Nat { 0 }\n",
        "fn k() -> Eq<Nat, 0, 0> -> Nat { sorry }\n",
        "fn inferred() { 0 }\n",
        "type Proof = Eq<Nat, 0, 0>\nfn data() -> Nat { 0 }\n",
        "type R = { n: Nat }\nfn r() -> R { sorry }\n",
        "fn p() -> Prop { sorry }\n",
    ];
    for source in free {
        assert_eq!(
            verdict_of(&[("f.tg", source)]),
            FileParse::ProofFree,
            "{source}"
        );
    }
}

/// 14.10.26b AC 1 (D2): an alias chain is followed across the files of one
/// tree, and the `fn`'s file alone does not see it.
#[test]
fn a_proof_type_is_followed_across_the_tree() {
    let fn_file = ("a.tg", "fn f() -> Result { sorry }\n");
    let tree = [
        fn_file,
        ("b.tg", "type Result = Proof\n"),
        ("c.tg", "type Proof = Eq<Nat, 0, 0>\n"),
    ];
    assert_eq!(verdict_of(&tree), FileParse::ProofBearing);
    assert_eq!(verdict_of(&tree[1..]), FileParse::ProofFree);
    assert_eq!(verdict_of(&[fn_file]), FileParse::ProofFree);
}

/// 14.10.26b AC 1 (D2): a self-referential or mutual `type` terminates, and
/// a cycle that reaches a proposition still carries one.
#[test]
fn a_type_cycle_terminates() {
    let alone = "type Loop = Loop\nfn l() -> Loop { sorry }\n";
    assert_eq!(verdict_of(&[("f.tg", alone)]), FileParse::ProofFree);
    let mutual = "type A = B\ntype B = (A, Nat)\nfn a() -> A { sorry }\n";
    assert_eq!(verdict_of(&[("f.tg", mutual)]), FileParse::ProofFree);
    let reaching = "type A = B\ntype B = (A, Void)\nfn a() -> A { sorry }\n";
    assert_eq!(verdict_of(&[("f.tg", reaching)]), FileParse::ProofBearing);
}

/// 14.10.26b D2: a file two trees share is proof-bearing when either tree's
/// types make it so, whichever tree is judged last. Trees are judged in path
/// order, so the proof-carrying root is named to sort first, then last.
#[test]
fn a_shared_file_is_proof_bearing_when_any_tree_says_so() {
    for (proof_root, plain_root) in [("a.tg", "b.tg"), ("b.tg", "a.tg")] {
        let dir = tempfile::tempdir().expect("tempdir");
        write(dir.path(), "shared.tg", "fn f() -> Proof { sorry }\n");
        write(dir.path(), proof_root, "type Proof = Eq<Nat, 0, 0>\n");
        write(dir.path(), plain_root, "type Proof = Nat\n");
        let file = |name: &str| dir.path().join(name);
        let mut plan = EntryPlan::default();
        for entry in [proof_root, plain_root] {
            plan.entries.push(file(entry));
            plan.trees
                .insert(file(entry), vec![file(entry), file("shared.tg")]);
        }
        let parses = parse_tree_files(&plan, parse_for_proofs);
        assert_eq!(
            parses[&file("shared.tg")],
            FileParse::ProofBearing,
            "{proof_root}"
        );
        assert_eq!(parses[&file(plain_root)], FileParse::ProofFree);
    }
}

/// 14.10.26b AC 2: an entry whose only proof is a false proof `fn` (13.10.26t
/// §1.1's `bad`) is census-ed under `--proof-bearing`, not skipped — before
/// this ADR its tree held no theorem and it read `skipped` — and the census
/// counts `bad` refused.
#[test]
fn a_proof_only_entry_is_census_ed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = fs::canonicalize(dir.path()).expect("canonical");
    write(
        &root,
        "proof/only.tg",
        "fn big() -> Nat { 18446744073709551615 }\n\
         fn bad() -> Eq<Nat, big() + 1, big()> { refl }\n\
         fn main() -> Nat { 0 }\n",
    );
    write(&root, "plain/data.tg", "fn main() -> Nat { 0 }\n");
    let report = census_corpus(&[root.clone()], true, &Register::EMPTY, &root).expect("a census");
    let outcome = |name: &str| {
        let found = report.files.iter().find(|(file, _)| file == name);
        found.expect("an entry").1.clone()
    };
    let FileOutcome::Censused(tally) = outcome("proof/only.tg") else {
        panic!("census-ed: {:?}", report.files);
    };
    assert_eq!((tally.proof_relevant, tally.refused), (1, 1));
    assert_eq!(outcome("plain/data.tg"), FileOutcome::Skipped);
}

/// 3.10.26s AC1 (D5): a file two trees share is parsed once; an unresolved
/// `mod` leaves its entry unbuilt.
#[test]
fn each_tree_file_is_parsed_once_and_an_unresolved_mod_is_unbuilt() {
    let plan = plan_of(&[("a", &["shared"]), ("b", &["shared"])]);
    let calls = Cell::new(0);
    let parses = parse_tree_files(&plan, |_: &Path| {
        calls.set(calls.get() + 1);
        Some(ProofFacts::default())
    });
    assert_eq!(parses.len(), 3);
    assert_eq!(calls.get(), 3);

    let dir = tempfile::tempdir().expect("tempdir");
    let root = fs::canonicalize(dir.path()).expect("canonical");
    write(&root, "lost.tg", "mod nowhere;\nfn main() -> Nat { 0 }\n");
    write(&root, "fine.tg", "fn main() -> Nat { 0 }\n");
    let plan = plan_entries(&walk_roots(&[root.clone()]).expect("walks"));
    assert_eq!(plan.unbuilt, BTreeSet::from([root.join("lost.tg")]));
    assert_eq!(plan.trees[&root.join("lost.tg")], [root.join("lost.tg")]);
}

/// 3.10.26s AC2: under the flag, an entry sharing a covered tree and a
/// proof-free entry are skipped, not empty; without it, nothing is.
#[test]
fn a_skipped_entry_is_not_empty() {
    let (_dir, root) = corpus();
    let report = census_corpus(&[root.clone()], true, &Register::EMPTY, &root).expect("a census");
    let outcome = |name: &str| {
        let found = report.files.iter().find(|(file, _)| file == name);
        found.expect("an entry").1.clone()
    };
    assert_eq!(outcome("plain/empty.tg"), FileOutcome::Skipped);
    assert_eq!(outcome("shared/b.tg"), FileOutcome::Skipped);
    assert!(matches!(outcome("shared/a.tg"), FileOutcome::Censused(_)));
    let rendered = render_corpus(&report, true);
    assert!(
        rendered.contains("6 files: 3 census-ed, 0 empty, 2 skipped, 1 not elaborating;"),
        "{rendered}"
    );
    assert!(rendered.contains("  plain/empty.tg: skipped (no proof item recognised in its tree)\n"));
    // The shared module is still census-ed, through the entry that kept it.
    assert_eq!(report.diff.verdict_line(), "register: MISMATCH (1 missing)");

    let without = census_corpus(&[root.clone()], false, &Register::EMPTY, &root).expect("a census");
    assert!(!without
        .files
        .iter()
        .any(|(_, o)| *o == FileOutcome::Skipped));
    let rendered = render_corpus(&without, false);
    assert!(
        rendered.contains("6 files: 4 census-ed, 1 empty, 1 not elaborating;"),
        "{rendered}"
    );
}

/// 3.10.26s AC3: a stale row in a file only a skipped entry reaches is not
/// examined, with the skipped reason — never matched, never absent — while a
/// row in a file a kept entry also reaches is still judged.
#[test]
fn a_skipped_file_is_not_examined() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = fs::canonicalize(dir.path()).expect("canonical");
    write(
        &root,
        "p/proof.tg",
        "theorem one(): 1 == 1 { refl }\nfn main() -> Nat { 0 }\n",
    );
    write(&root, "q/plain.tg", "fn main() -> Nat { 0 }\n");
    let ghost = register(&needs_delta_row("q/plain.tg", "ghost"));
    let report = census_corpus(&[root.clone()], true, &ghost, &root).expect("a census");
    assert_eq!(
        categories(&report.diff),
        [DiffCategory::NotExamined(NotExaminedWhy::Skipped)]
    );
    assert_eq!(
        report.diff.lines[0].detail,
        "skipped: no proof item recognised"
    );
    assert_eq!(
        report.diff.verdict_line(),
        "register: INCOMPLETE (1 row(s) not examined)"
    );
    // Without the flag the file is census-ed, so the stale row is absent.
    let report = census_corpus(&[root.clone()], false, &ghost, &root).expect("a census");
    assert_eq!(categories(&report.diff), [DiffCategory::Absent]);

    let (_dir, root) = corpus();
    let gone = register(&needs_delta_row("shared/common.tg", "gone"));
    let report = census_corpus(&[root.clone()], true, &gone, &root).expect("a census");
    assert!(
        categories(&report.diff).contains(&DiffCategory::Absent),
        "{:?}",
        report.diff
    );
}
