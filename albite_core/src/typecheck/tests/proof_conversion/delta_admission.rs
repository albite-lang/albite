//! ADR 2.10.26l D3, D4: `DeltaAdmission`'s own contracts — the mention set
//! it closes over, the context a body is checked in, the binder a
//! self-recursive body is abstracted to, and the positions `admit_all`
//! reports. Each pins one decision the AC 1 suite only reaches end to end.

use std::collections::BTreeSet;

use super::*;
use crate::conversion::Unfoldings;
use crate::terms::termination::tests::fixtures::{length_body, list_to_nat};
use crate::typecheck::{mentioned_names, present_definition, DeltaAdmission, DeltaCandidate};

fn candidate<'a>(ty: &'a Type, body: &'a Term, total: bool) -> DeltaCandidate<'a> {
    DeltaCandidate {
        ty,
        body,
        total,
        decreasing: None,
    }
}

// D3.4: the closure follows names the `Eq` witnesses of a type spell, not
// only the term's own nodes.
#[test]
fn mentioned_names_reads_the_witnesses_of_the_type() {
    let statement = eq_nat(Term::global("two"), Term::NatLit(2));
    let names = mentioned_names(&Term::Unit, &statement);
    assert_eq!(names, BTreeSet::from(["two".to_string()]));
    let annotated = Term::annot(Term::Unit, eq_nat(Term::global("three"), Term::Zero));
    assert!(mentioned_names(&annotated, &Type::Unit).contains("three"));
}

// D3.3: a body is checked with each global it mentions bound by the report's
// marker, so a body that cites a `Partial` global is refused, and is never
// admitted even though its own marker says `Total`.
#[test]
fn a_body_citing_a_partial_global_is_not_admitted() {
    let spin_body = Term::NatLit(2);
    let wrapper_body = Term::global("spin");
    let mut admission = DeltaAdmission::new([
        ("spin".to_string(), candidate(&Type::Nat, &spin_body, false)),
        (
            "wrapper".to_string(),
            candidate(&Type::Nat, &wrapper_body, true),
        ),
    ]);
    let roots = BTreeSet::from(["wrapper".to_string()]);
    let ctx = admission.bind(&Context::new(), &roots);
    assert!(ctx.unfoldings().body("wrapper").is_none());
    assert!(admission.admit_all().names().is_empty());
}

// D4: the self-call is abstracted to a fresh binder derived from the name,
// never one the body already spells.
#[test]
fn present_definition_abstracts_to_a_fresh_self_binder() {
    let body = length_body(Term::global("length"));
    let Term::Fix(binder, _, abstracted) = present_definition("length", &list_to_nat(), &body)
    else {
        panic!("a self-recursive body is presented as a fix");
    };
    assert!(binder.starts_with("length%self"), "{binder}");
    assert!(!mentioned_names(&body, &Type::Unit).contains(&binder));
    assert!(mentioned_names(&abstracted, &Type::Unit).contains(&binder));
    assert!(!mentioned_names(&abstracted, &Type::Unit).contains("length"));
    // A body that already spells the first candidate gets another.
    let taken = Term::pair(body.clone(), Term::var(binder.clone()));
    let Term::Fix(other, ..) = present_definition("length", &list_to_nat(), &taken) else {
        panic!("still a fix");
    };
    assert_ne!(other, binder);
    // A body that never calls itself is presented as itself.
    assert_eq!(
        present_definition("two", &Type::Nat, &Term::NatLit(2)),
        Term::NatLit(2)
    );
}

// D4: `admit_all` reports a fixpoint's certified position, and none for a
// body that is not a fixpoint.
#[test]
fn admit_all_reports_each_fixpoints_position() {
    let length = length_body(Term::global("length"));
    let two = Term::NatLit(2);
    let list_to_nat = list_to_nat();
    let mut admission = DeltaAdmission::new([
        ("length".to_string(), candidate(&list_to_nat, &length, true)),
        ("two".to_string(), candidate(&Type::Nat, &two, true)),
    ]);
    let admitted = admission.admit_all();
    assert_eq!(admitted.decreasing("length"), Some(0));
    assert_eq!(admitted.decreasing("two"), None);
    assert!(matches!(admitted.body("length"), Some(Term::Fix(..))));
    assert_eq!(admitted.body("two"), Some(&two));
}
