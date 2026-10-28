//! ADR 3.10.26o D4's must-fail twins. Each calls `type_of_dependent_case`
//! directly and asserts its own refusal: through `type_of`, D2 turns every
//! refusal into the old rule's error, so a twin there could pass for the
//! wrong reason.

use super::*;

fn refused_because(ctx: &Context, split: &Term, goal: &Type) -> &'static str {
    match type_of_dependent_case(ctx, split, goal) {
        Err(TypeError::DependentCaseShape { why }) => why,
        other => panic!("expected a shape refusal, got {other:?}"),
    }
}

/// An `Opt` split whose `inr` arm body is `body`.
fn opt_split_with_some_arm(scrutinee: Term, body: Term) -> Term {
    Term::case(scrutinee, "u", Term::refl(opt(), none()), "b", body)
}

fn some_arm(fields: &[(&str, Type)], proof: Term) -> Term {
    chain("b", fields, proof)
}

/// 3.10.26o AC1: an arm proving another constructor's instance is refused.
#[test]
fn an_arm_proving_the_wrong_instance_is_refused() {
    let split = opt_split_with_some_arm(
        Term::var("o"),
        some_arm(&[("x", Type::Nat)], Term::refl(opt(), none())),
    );
    let verdict = type_of_dependent_case(&proof_over(&[("o", opt())]), &split, &rebuild_goal("o"));
    assert!(
        matches!(verdict, Err(TypeError::TypeMismatch { .. })),
        "{verdict:?}"
    );
}

fn tri_match(indices: &[usize]) -> Term {
    let arms = indices
        .iter()
        .map(|index| {
            let body = Term::refl(tri(), Term::adt_construct(tri(), 0, Term::Unit));
            Term::adt_arm(*index, "a", body)
        })
        .collect();
    Term::adt_match(Term::var("t"), arms)
}

/// 3.10.26o AC1: `adt_match t [0 => refl] : Eq Tri t Lo` would prove a false
/// statement under a dependent motive; a missing index is refused.
#[test]
fn a_missing_adt_index_is_refused() {
    let lo = Term::adt_construct(tri(), 0, Term::Unit);
    let goal = Type::eq(tri(), Term::var("t"), lo);
    let ctx = proof_over(&[("t", tri())]);
    assert_eq!(
        refused_because(&ctx, &tri_match(&[0]), &goal),
        "a constructor has no arm"
    );
}

/// 3.10.26o AC1: a constructor covered twice is refused.
#[test]
fn a_duplicate_adt_index_is_refused() {
    let ctx = proof_over(&[("t", tri())]);
    let goal = reflexive_goal(tri(), "t");
    let why = refused_because(&ctx, &tri_match(&[0, 1, 1, 2]), &goal);
    assert_eq!(why, "a constructor has two arms");
    let why = refused_because(&ctx, &tri_match(&[0, 1, 2, 3]), &goal);
    assert_eq!(why, "an arm names no constructor");
}

/// 3.10.26o AC1: no chain over a `Nat` payload binds fewer fields than it has.
#[test]
fn a_chain_shorter_than_the_payload_is_refused() {
    let split = opt_split_with_some_arm(Term::var("o"), Term::refl(opt(), none()));
    let why = refused_because(&proof_over(&[("o", opt())]), &split, &rebuild_goal("o"));
    assert_eq!(why, "an arm with no fields needs a Unit payload");
}

/// 3.10.26o AC1: three fields over `Nat × Lst` asks for a component it lacks.
#[test]
fn a_chain_longer_than_the_payload_is_refused() {
    let fields = [("h", Type::Nat), ("t", Type::Nat), ("r", lst())];
    let split = Term::case(
        Term::unfold(lst(), Term::var("l")),
        "u",
        Term::refl(Type::Nat, Term::Zero),
        "b",
        chain("b", &fields, Term::refl(Type::Nat, Term::Zero)),
    );
    let why = refused_because(
        &proof_over(&[("l", lst())]),
        &split,
        &reflexive_goal(lst(), "l"),
    );
    assert_eq!(why, "the payload has fewer components than the chain");
}

/// 3.10.26o AC1: the chain must apply the binder's projections, in order.
#[test]
fn an_argument_other_than_the_projection_is_refused() {
    let body = Term::app(
        Term::lambda("x", Type::Nat, Term::refl(opt(), some(Term::var("x")))),
        Term::Zero,
    );
    let split = opt_split_with_some_arm(Term::var("o"), body);
    let why = refused_because(&proof_over(&[("o", opt())]), &split, &rebuild_goal("o"));
    assert_eq!(why, "an argument is not the binder's projection");
}

/// 3.10.26o AC1: over a two-field payload, the first argument must be a
/// `fst` and the second a `snd` — swapping or dropping either is no chain.
#[test]
fn a_two_field_chain_with_the_wrong_projections_is_refused() {
    let split_with = |first: Term, second: Term| {
        let function = Term::lambda(
            "h",
            Type::Nat,
            Term::lambda("t", lst(), Term::refl(Type::Nat, Term::Zero)),
        );
        let body = Term::app(Term::app(function, first), second);
        Term::case(
            Term::unfold(lst(), Term::var("l")),
            "u",
            Term::refl(Type::Nat, Term::Zero),
            "b",
            body,
        )
    };
    let ctx = proof_over(&[("l", lst())]);
    let goal = reflexive_goal(lst(), "l");
    let binder = || Term::var("b");
    for split in [
        split_with(Term::snd(binder()), Term::snd(binder())),
        split_with(Term::fst(binder()), Term::fst(binder())),
    ] {
        assert_eq!(
            refused_because(&ctx, &split, &goal),
            "an argument is not the binder's projection"
        );
    }
}

/// 3.10.26o AC1: the refusal renders its reason, for anyone who prints it.
#[test]
fn a_shape_refusal_renders_its_reason() {
    let rendered = TypeError::DependentCaseShape {
        why: "a binder shadows the scrutinee",
    }
    .to_string();
    assert_eq!(
        rendered,
        "not a dependent case split: a binder shadows the scrutinee"
    );
}

/// 3.10.26o AC1: two applications of a one-λ head is fewer λs than projections.
#[test]
fn fewer_lambdas_than_projections_are_refused() {
    let one_lambda = Term::lambda("h", Type::Nat, Term::refl(Type::Nat, Term::Zero));
    let body = Term::app(
        Term::app(one_lambda, Term::fst(Term::var("b"))),
        Term::snd(Term::var("b")),
    );
    let split = Term::case(
        Term::unfold(lst(), Term::var("l")),
        "u",
        Term::refl(Type::Nat, Term::Zero),
        "b",
        body,
    );
    let why = refused_because(
        &proof_over(&[("l", lst())]),
        &split,
        &reflexive_goal(lst(), "l"),
    );
    assert_eq!(why, "fewer λs than projections");
}

/// 3.10.26o AC1: a binder typed unlike its payload component is refused.
#[test]
fn a_binder_type_unlike_the_payload_is_refused() {
    let split = opt_split_with_some_arm(Term::var("o"), some_arm(&[("x", Type::Bool)], Term::True));
    let why = refused_because(&proof_over(&[("o", opt())]), &split, &rebuild_goal("o"));
    assert_eq!(why, "a binder's type is not its payload component");
}

/// 3.10.26o AC1: a binder free in the goal would be captured by the instance.
#[test]
fn a_binder_capturing_a_goal_variable_is_refused() {
    let ctx = proof_over(&[("o", opt()), ("b", Type::Nat)]);
    let goal = Type::eq(Type::Nat, Term::var("b"), Term::var("b"));
    let why = refused_because(&ctx, &opt_split("o"), &goal);
    assert_eq!(why, "a binder occurs free in the goal");
}

/// 3.10.26o AC1: a field free in the goal only under an inner λ is caught too.
#[test]
fn a_binder_captured_under_an_inner_lambda_is_refused() {
    let ctx = proof_over(&[("o", opt()), ("x", Type::Nat)]);
    let constant = Term::lambda("y", Type::Nat, Term::var("x"));
    let goal = Type::eq(
        Type::arrow(Type::Nat, Type::Nat),
        constant.clone(),
        constant,
    );
    let why = refused_because(&ctx, &opt_split("o"), &goal);
    assert_eq!(why, "a binder occurs free in the goal");
}

/// 3.10.26o AC1: a field named after the scrutinee, or after any other
/// binding of the context, would make a hypothesis read the field.
#[test]
fn a_binder_shadowing_the_scrutinee_is_refused() {
    let shadowing = |field: &str| {
        opt_split_with_some_arm(
            Term::var("o"),
            some_arm(
                &[(field, Type::Nat)],
                Term::refl(opt(), some(Term::var(field))),
            ),
        )
    };
    let nil = Term::inl(opt(), Term::Unit);
    let ctx = proof_over(&[("o", opt()), ("h", Type::eq(opt(), Term::var("o"), nil))]);
    let goal = rebuild_goal("o");
    assert_eq!(
        refused_because(&ctx, &shadowing("o"), &goal),
        "a binder shadows the scrutinee"
    );
    assert_eq!(
        refused_because(&ctx, &shadowing("h"), &goal),
        "a binder shadows a binding of the context"
    );
}

/// 3.10.26o AC1: two fields of one arm sharing a name would let the proof
/// speak of one where the instance names both.
#[test]
fn two_fields_sharing_a_name_are_refused() {
    let split = Term::case(
        Term::unfold(lst(), Term::var("l")),
        "u",
        Term::refl(Type::Nat, Term::Zero),
        "b",
        chain(
            "b",
            &[("z", Type::Nat), ("z", lst())],
            Term::refl(Type::Nat, Term::Zero),
        ),
    );
    let why = refused_because(
        &proof_over(&[("l", lst())]),
        &split,
        &reflexive_goal(lst(), "l"),
    );
    assert_eq!(why, "two fields of one arm share a name");
}

/// 3.10.26o AC1: only a variable is abstracted, never a compound term.
#[test]
fn a_compound_scrutinee_is_refused() {
    let ctx = proof_over(&[("o", opt())]);
    let split = opt_split_with_some_arm(none(), some_arm(&[("x", Type::Nat)], Term::Unit));
    let why = refused_because(&ctx, &split, &rebuild_goal("o"));
    assert_eq!(why, "the scrutinee is not a variable");
    let unfolded = Term::case(
        Term::unfold(lst(), Term::fold(lst(), Term::inl(lst_body(), Term::Unit))),
        "u",
        Term::Unit,
        "b",
        Term::Unit,
    );
    assert_eq!(
        refused_because(&ctx, &unfolded, &rebuild_goal("o")),
        "the scrutinee is not a variable"
    );
}

/// 3.10.26o AC1: a global's value is not a variable the motive may abstract.
#[test]
fn a_global_scrutinee_is_refused() {
    let split = opt_split("g");
    let why = refused_because(&with_admitted_global("g"), &split, &rebuild_goal("g"));
    assert_eq!(why, "the scrutinee is not a term binding of the context");
}

/// 3.10.26o AC1: a case on a `Nat`, or an `adt_match` on a sum, is no split.
#[test]
fn a_non_sum_variable_is_refused() {
    let ctx = proof_over(&[("n", Type::Nat), ("o", opt())]);
    let goal = reflexive_goal(Type::Nat, "n");
    assert_eq!(
        refused_because(&ctx, &opt_split("n"), &goal),
        "a case scrutinee that is not a sum"
    );
    let on_sum = Term::adt_match(Term::var("o"), vec![Term::adt_arm(0, "a", Term::Unit)]);
    assert_eq!(
        refused_because(&ctx, &on_sum, &goal),
        "an adt_match scrutinee that is not an ADT"
    );
    assert_eq!(
        refused_because(&ctx, &Term::var("o"), &goal),
        "not a case or adt_match"
    );
}

/// 3.10.26o AC1: executable mode never splits.
#[test]
fn executable_mode_does_not_split() {
    let ctx = Context::new().with_term("o", opt());
    let why = refused_because(&ctx, &opt_split("o"), &rebuild_goal("o"));
    assert_eq!(why, "executable mode does not split");
    let annotated = Term::annot(opt_split("o"), rebuild_goal("o"));
    assert!(type_of(&ctx, &annotated).is_err());
}

/// 3.10.26o AC1: a poisoned scrutinee, or a poisoned payload a chain reads
/// into, is the proof judgment's poison refusal, never a split.
#[test]
fn a_poisoned_scrutinee_or_payload_is_refused() {
    let poison = Err(TypeError::PoisonInProof { ty: Type::Error });
    let ctx = proof_over(&[("p", Type::Error)]);
    let goal = reflexive_goal(Type::Nat, "p");
    assert_eq!(type_of_dependent_case(&ctx, &opt_split("p"), &goal), poison);

    let ctx = proof_over(&[("q", Type::sum(Type::Unit, Type::Error))]);
    let split = Term::case(
        Term::var("q"),
        "u",
        Term::Unit,
        "b",
        chain("b", &[("h", Type::Nat), ("t", Type::Nat)], Term::Unit),
    );
    assert_eq!(type_of_dependent_case(&ctx, &split, &goal), poison);
}
