//! Tests for `rests_on` and the shared `is_axiom` (ADR 13.10.26g AC2).

use std::collections::{BTreeMap, BTreeSet};

use super::{is_axiom, rests_on, rests_on_refs, RootClass};
use crate::conversion::delta::global_names;
use crate::terms::{Term, TermSpan};
use crate::types::Type;

const AXIOM_ITEM_SPAN: TermSpan = TermSpan::new(0, 40);

/// An axiom's body: a `sorry` spanned by the whole item.
fn axiom_body() -> Term {
    Term::spanned(Term::Sorry, AXIOM_ITEM_SPAN)
}

/// An authored `{ sorry }`: spanned by the token, not the item.
fn authored_hole() -> Term {
    Term::spanned(Term::Sorry, TermSpan::new(30, 35))
}

fn global(name: &str) -> Term {
    Term::Global(name.to_string())
}

fn cites(names: &[&str]) -> Term {
    names
        .iter()
        .map(|name| global(name))
        .reduce(|left, right| Term::App(Box::new(left), Box::new(right)))
        .unwrap_or(Term::Unit)
}

/// A match whose second arm is a hole the pattern lowering planted.
fn synthesised_hole() -> Term {
    Term::adt_match(
        Term::Unit,
        vec![
            (0, "x".to_string(), Box::new(Term::Zero)),
            (1, "__unreachable_1".to_string(), Box::new(Term::Sorry)),
        ],
    )
}

/// The roots of a project, classified the way the bootstrap classifies them.
///
/// Every case also runs the reference-set closure and asserts it agrees, so
/// each existing case doubles as an agreement check (ADR 16.10.26g AC2).
fn closure(defs: &[(&str, Term)]) -> BTreeMap<String, BTreeSet<String>> {
    let roots: BTreeSet<&str> = defs
        .iter()
        .filter(|(_, term)| RootClass::of(term, Some(AXIOM_ITEM_SPAN)).is_some())
        .map(|(name, _)| *name)
        .collect();
    let borrowed: Vec<(&str, &Term)> = defs.iter().map(|(name, term)| (*name, term)).collect();
    let from_terms = rests_on(&borrowed, |name| roots.contains(name));
    let sets: Vec<(&str, BTreeSet<String>)> = defs
        .iter()
        .map(|(name, term)| (*name, global_names(term)))
        .collect();
    let refs: Vec<(&str, &BTreeSet<String>)> =
        sets.iter().map(|(name, cited)| (*name, cited)).collect();
    let from_refs = rests_on_refs(&refs, |name| roots.contains(name));
    assert_eq!(
        from_refs, from_terms,
        "rests_on_refs disagrees with rests_on"
    );
    from_terms
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

// 13.10.26g AC2: one hop — `bad2` cites `ax`.
#[test]
fn rests_on_one_hop_names_the_axiom() {
    let found = closure(&[("ax", axiom_body()), ("bad2", cites(&["ax"]))]);
    assert_eq!(found.get("bad2"), Some(&set(&["ax"])));
}

// 13.10.26g AC2: two hops — `bad3` cites `bad2`, which cites `ax`.
#[test]
fn rests_on_two_hops_still_reaches_the_axiom() {
    let found = closure(&[
        ("ax", axiom_body()),
        ("bad2", cites(&["ax"])),
        ("bad3", Term::Absurd(Type::Void, Box::new(cites(&["bad2"])))),
    ]);
    assert_eq!(found.get("bad3"), Some(&set(&["ax"])));
}

// 13.10.26g AC2: a `fn` calling the axiom is walked through.
#[test]
fn rests_on_walks_through_a_program_definition() {
    let found = closure(&[
        ("ax", axiom_body()),
        ("helper", Term::Succ(Box::new(cites(&["ax"])))),
        ("uses_helper", cites(&["helper"])),
    ]);
    assert_eq!(found.get("uses_helper"), Some(&set(&["ax"])));
}

// 13.10.26g AC2: a theorem whose proof is `sorry` is a root of its citers.
#[test]
fn rests_on_an_authored_hole_names_its_definition() {
    let found = closure(&[
        ("holey", authored_hole()),
        ("uses_holey", cites(&["holey"])),
    ]);
    assert_eq!(found.get("uses_holey"), Some(&set(&["holey"])));
    assert!(!found.contains_key("holey"), "{found:?}");
}

// 13.10.26g AC2: mutual recursion terminates and still reaches the root.
#[test]
fn rests_on_a_cycle_terminates() {
    let found = closure(&[
        ("ax", axiom_body()),
        ("even", cites(&["odd", "even"])),
        ("odd", cites(&["even", "ax"])),
    ]);
    assert_eq!(found.get("even"), Some(&set(&["ax"])));
    assert_eq!(found.get("odd"), Some(&set(&["ax"])));
}

// 13.10.26g AC2: a definition whose only hole is synthesised is no root.
#[test]
fn rests_on_a_synthesised_hole_is_not_a_root() {
    assert_eq!(
        RootClass::of(&synthesised_hole(), Some(AXIOM_ITEM_SPAN)),
        None
    );
    let found = closure(&[
        ("matcher", synthesised_hole()),
        ("uses", cites(&["matcher"])),
    ]);
    assert!(found.is_empty(), "{found:?}");
}

// 13.10.26g D2: an unclassified hole is a real gap, so it is a root.
#[test]
fn rests_on_an_unclassified_hole_is_a_root() {
    let unclassified = Term::Succ(Box::new(Term::Sorry));
    assert_eq!(RootClass::of(&unclassified, None), Some(RootClass::Sorry));
    let found = closure(&[("gap", unclassified), ("uses_gap", cites(&["gap"]))]);
    assert_eq!(found.get("uses_gap"), Some(&set(&["gap"])));
}

// 13.10.26g AC2: the clean control rests on nothing.
#[test]
fn rests_on_a_clean_citation_is_absent() {
    let found = closure(&[("fine", Term::Unit), ("uses_fine", cites(&["fine"]))]);
    assert!(found.is_empty(), "{found:?}");
}

// 13.10.26g AC2: an axiom alone rests on nothing and is not listed.
#[test]
fn rests_on_an_axiom_alone_is_not_listed() {
    let found = closure(&[("ax", axiom_body())]);
    assert!(found.is_empty(), "{found:?}");
}

// 13.10.26g D1: every root reached is named, and a non-root name is not.
#[test]
fn rests_on_names_every_root_and_only_roots() {
    let found = closure(&[
        ("ax", axiom_body()),
        ("holey", authored_hole()),
        ("fine", Term::Unit),
        ("both", cites(&["ax", "holey", "fine", "unknown"])),
    ]);
    assert_eq!(found.get("both"), Some(&set(&["ax", "holey"])));
}

// 13.10.26g D1: a root outside `defs` is named but not expanded.
#[test]
fn rests_on_a_root_outside_the_population_is_named() {
    let defs = [("bad2", cites(&["imported_ax"]))];
    let borrowed: Vec<(&str, &Term)> = defs.iter().map(|(name, term)| (*name, term)).collect();
    let found = rests_on(&borrowed, |name| name == "imported_ax");
    assert_eq!(found.get("bad2"), Some(&set(&["imported_ax"])));
}

// 13.10.26g D2: the axiom shape test the kernel census used, now shared.
#[test]
fn is_axiom_reads_through_binders_and_wants_the_item_span() {
    let under_binders = Term::TyAbs(
        "T".into(),
        Box::new(Term::Lambda(
            "x".into(),
            Type::Nat,
            Box::new(Term::Annot(Box::new(axiom_body()), Type::Nat)),
        )),
    );
    assert!(is_axiom(&under_binders, AXIOM_ITEM_SPAN));
    // Spanned by the item but holding no `sorry`: an ordinary body, not an axiom.
    assert!(!is_axiom(
        &Term::spanned(Term::Unit, AXIOM_ITEM_SPAN),
        AXIOM_ITEM_SPAN
    ));
    assert!(is_axiom(
        &Term::spanned(axiom_body(), AXIOM_ITEM_SPAN),
        AXIOM_ITEM_SPAN
    ));
    assert!(!is_axiom(&authored_hole(), AXIOM_ITEM_SPAN));
    assert!(!is_axiom(&Term::Unit, AXIOM_ITEM_SPAN));
    assert_eq!(
        RootClass::of(&axiom_body(), Some(AXIOM_ITEM_SPAN)),
        Some(RootClass::Axiom)
    );
    assert_eq!(RootClass::of(&axiom_body(), None), Some(RootClass::Sorry));
    assert_eq!(
        RootClass::of(&authored_hole(), Some(AXIOM_ITEM_SPAN)),
        Some(RootClass::Sorry)
    );
    assert_eq!(RootClass::of(&Term::Unit, Some(AXIOM_ITEM_SPAN)), None);
    assert_eq!(RootClass::Axiom.label(), "axiom");
    assert_eq!(RootClass::Sorry.label(), "sorry");
}

// 16.10.26g AC2: a name with no entry — an import the stream never saw — is
// still tested as a root, but its own references are not followed.
#[test]
fn rests_on_refs_tests_an_unlisted_name_without_expanding_it() {
    let cites_external = set(&["external"]);
    let refs = [("thm", &cites_external)];
    let resting = rests_on_refs(&refs, |name| name == "external");
    assert_eq!(
        resting,
        BTreeMap::from([("thm".to_string(), set(&["external"]))])
    );
    let not_root = rests_on_refs(&refs, |_| false);
    assert!(not_root.is_empty(), "{not_root:?}");
}

// 16.10.26g AC2: a repeated name keeps its last reference set.
#[test]
fn rests_on_refs_keeps_the_last_set_of_a_repeated_name() {
    let first = set(&["ax"]);
    let last = set(&[]);
    let refs = [("thm", &first), ("thm", &last)];
    assert!(rests_on_refs(&refs, |name| name == "ax").is_empty());
}
