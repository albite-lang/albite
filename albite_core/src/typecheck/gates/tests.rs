//! The gates as pure functions (ADR 29.9.26e §2.1), and the P0.2/P0.3
//! measurements the ADR's §6 records.

use super::*;
use crate::terms::termination::tests::fixtures::{list_fn, list_type, match_list, var};

fn tv(name: &str) -> Type {
    Type::TyVar(name.into())
}

fn refused_mu(mu: &Type) -> bool {
    matches!(
        mu_positivity(mu),
        Err(TypeError::NonStrictlyPositiveMu { .. })
    )
}

// ==========================================================================
// P0.2 — the singleton presentation of a μ (D6)
// ==========================================================================

#[test]
fn p0_2_positivity_refuses_the_negative_shapes() {
    let curry = Type::mu("a", Type::arrow(tv("a"), Type::Void));
    let alpha_prefixed = Type::mu("α_B", Type::arrow(tv("B"), Type::Void));
    let at_prefixed = Type::mu("α_B", Type::arrow(tv("@B"), Type::Void));
    let inline_adt = Type::mu(
        "a",
        Type::Adt(
            "Bad".into(),
            vec![],
            vec![("Mk".into(), Type::arrow(tv("a"), Type::Void))],
        ),
    );
    let deep = Type::mu("a", Type::arrow(Type::arrow(tv("a"), Type::Nat), Type::Nat));
    for mu in [curry, alpha_prefixed, at_prefixed, inline_adt, deep] {
        assert!(refused_mu(&mu), "{mu}");
    }
}

#[test]
fn p0_2_positivity_accepts_the_positive_shapes() {
    let nat_to = Type::mu("a", Type::arrow(Type::Nat, tv("a")));
    let inline_positive = Type::mu(
        "a",
        Type::Adt(
            "Tree".into(),
            vec![],
            vec![("Node".into(), Type::product(tv("a"), tv("a")))],
        ),
    );
    for mu in [list_type(), nat_to, inline_positive] {
        assert_eq!(mu_positivity(&mu), Ok(()), "{mu}");
        assert!(!refused_mu(&mu), "{mu}");
    }
    // Not a μ at all: the rules report the malformed type themselves.
    assert_eq!(mu_positivity(&Type::arrow(tv("a"), Type::Void)), Ok(()));
}

/// Recorded by P0.2: the bootstrap encodes `TyApp(TypeExpr, List<TypeExpr>)`
/// (`src/compiler/main.tg`) with `List` inlined as a nested `μα_List` inside
/// an inline `Adt`, not as an `App` — a shape the gate accepts, including
/// the outer binder's bare spelling reused as the `Adt`'s name.
#[test]
fn p0_2_the_bootstrap_encoding_of_a_nested_list_is_accepted() {
    let inner_list = Type::mu(
        "α_List",
        Type::sum(Type::Unit, Type::product(tv("α_TypeExpr"), tv("α_List"))),
    );
    let mu = Type::mu(
        "α_TypeExpr",
        Type::mu(
            "α_Expr",
            Type::Adt(
                "TypeExpr".into(),
                vec![],
                vec![
                    (
                        "TyArrow".into(),
                        Type::product(tv("α_TypeExpr"), tv("α_TypeExpr")),
                    ),
                    ("TyApp".into(), Type::product(tv("α_TypeExpr"), inner_list)),
                    ("TyEq".into(), Type::product(tv("α_Expr"), tv("α_Expr"))),
                ],
            ),
        ),
    );
    assert_eq!(mu_positivity(&mu), Ok(()));
}

/// Recorded by P0.2: a nested inductive over an unknown head is refused —
/// conservative, and accepted as a Non-Goal.
#[test]
fn p0_2_nested_inductive_over_an_unknown_head_is_refused() {
    let nested = Type::mu(
        "a",
        Type::Adt(
            "Node".into(),
            vec![],
            vec![("Node".into(), Type::App("List".into(), vec![tv("a")]))],
        ),
    );
    assert!(refused_mu(&nested));
}

#[test]
fn the_refusal_names_the_mu_and_the_presented_field() {
    let mu = Type::mu("a", Type::arrow(tv("a"), Type::Void));
    let Err(TypeError::NonStrictlyPositiveMu { mu: named, field }) = mu_positivity(&mu) else {
        panic!("expected a refusal");
    };
    assert_eq!(named, mu);
    assert!(field.contains("mu#a"), "{field}");
    assert!(field.contains("Void"), "{field}");
}

#[test]
fn every_spelling_of_the_binder_is_presented_as_the_member() {
    let body = Type::product(
        Type::product(tv("α_B"), tv("B")),
        Type::product(tv("@B"), tv("C")),
    );
    let presented = present_mu_field("α_B", &body, "g");
    let expected = Type::product(
        Type::product(tv("g"), tv("g")),
        Type::product(tv("g"), tv("C")),
    );
    assert_eq!(presented, expected);
    // An unprefixed binder maps itself and its `@` spelling.
    let plain = present_mu_field("a", &Type::product(tv("a"), tv("@a")), "g");
    assert_eq!(plain, Type::product(tv("g"), tv("g")));
}

#[test]
fn the_presented_member_name_is_fresh() {
    let binder = "a";
    assert_eq!(fresh_type_name(binder, &Type::Nat), "mu#a");
    let taken = Type::product(tv("mu#a"), Type::App("mu#a#1".into(), vec![]));
    assert_eq!(fresh_type_name(binder, &taken), "mu#a#2");
    for spelling in [
        Type::mu("mu#a", Type::Nat),
        Type::forall("mu#a", Type::Nat),
        Type::Adt("mu#a".into(), vec![], vec![]),
        Type::arrow(Type::Nat, tv("mu#a")),
    ] {
        assert!(type_mentions_name(&spelling, "mu#a"), "{spelling}");
    }
    assert!(!type_mentions_name(&tv("other"), "mu#a"));
}

// ==========================================================================
// P0.3 — the one-definition presentation of a fixpoint (D3)
// ==========================================================================

fn structural_len_body() -> Term {
    list_fn(
        "l",
        match_list(
            var("l"),
            Term::Zero,
            "t",
            Term::app(Term::var("len"), var("t")),
        ),
    )
}

#[test]
fn p0_3_structural_len_is_certified_total() {
    let ty = Type::arrow(list_type(), Type::Nat);
    assert_eq!(
        fix_admission("len", &ty, &structural_len_body(), &[], None),
        Ok(0)
    );
}

#[test]
fn p0_3_a_self_loop_is_refused_with_a_reason() {
    let Err(TypeError::FixNotStructural { name, reason }) =
        fix_admission("f", &Type::Void, &Term::var("f"), &[], None)
    else {
        panic!("expected a refusal");
    };
    assert_eq!(name, "f");
    // A bare self-reference is not a call, so the engine cannot resolve it
    // into a call-graph edge at all.
    assert!(reason.contains("IndirectOccurrence"), "{reason}");
    assert!(reason.contains("fix#f"), "{reason}");
}

/// Taint cannot see an absent global, and the occurrence graph adds no edge
/// to one: a body calling a global outside the presented set is judged on
/// its own recursion alone. That includes a body that already calls a global
/// spelled `fix#<name>`: the presented name is fresh, so the call is not
/// mistaken for recursion.
#[test]
fn p0_3_an_absent_global_does_not_reject() {
    for (fixpoint, called) in [("f", "elsewhere"), ("len", "fix#len")] {
        let body = Term::lambda(
            "l",
            list_type(),
            Term::app(Term::global(called), Term::var("l")),
        );
        let ty = Type::arrow(list_type(), Type::Nat);
        assert_eq!(
            fix_admission(fixpoint, &ty, &body, &[], None),
            Ok(0),
            "{called}"
        );
    }
}

/// `xs ys ↦ len xs (tail ys)`: two list parameters, descending on the second.
fn second_parameter_len_body(decreasing_name: &str) -> Term {
    list_fn(
        "xs",
        list_fn(
            "ys",
            match_list(
                var(decreasing_name),
                Term::Zero,
                "t",
                if decreasing_name == "ys" {
                    Term::app(Term::app(Term::var("len"), var("xs")), var("t"))
                } else {
                    Term::app(Term::app(Term::var("len"), var("t")), var("ys"))
                },
            ),
        ),
    )
}

// ADR 2.10.26l D4: the certified decreasing position comes back, and an
// annotation is a hint the engine re-checks, never trusts.
#[test]
fn fix_admission_returns_the_certified_position() {
    let ty = Type::arrow(list_type(), Type::arrow(list_type(), Type::Nat));
    assert_eq!(
        fix_admission("len", &ty, &second_parameter_len_body("ys"), &[], None),
        Ok(1)
    );
    assert_eq!(
        fix_admission("len", &ty, &second_parameter_len_body("xs"), &[], None),
        Ok(0)
    );
    assert_eq!(
        fix_admission(
            "len",
            &ty,
            &second_parameter_len_body("ys"),
            &[],
            Some("ys")
        ),
        Ok(1)
    );
    assert!(matches!(
        fix_admission(
            "len",
            &ty,
            &second_parameter_len_body("ys"),
            &[],
            Some("xs")
        ),
        Err(TypeError::FixNotStructural { .. })
    ));
}

/// The type variables in scope are presented as the definition's own type
/// parameters, so a descent on an abstract `a` is not certified — and the
/// same body, with `a` not presented, would be.
#[test]
fn p0_3_type_parameters_are_not_descent_roots() {
    let body = Term::lambda(
        "x",
        tv("a"),
        Term::app(Term::var("f"), Term::snd(Term::var("x"))),
    );
    let ty = Type::arrow(tv("a"), Type::Nat);
    assert!(matches!(
        fix_admission("f", &ty, &body, &["a".to_string()], None),
        Err(TypeError::FixNotStructural { .. })
    ));
    assert_eq!(fix_admission("f", &ty, &body, &[], None), Ok(0));
}

#[test]
fn the_presented_global_is_fresh() {
    assert_eq!(fresh_global_name("f", &Term::Unit), "fix#f");
    let taken = Term::pair(Term::global("fix#f"), Term::global("fix#f#1"));
    assert_eq!(fresh_global_name("f", &taken), "fix#f#2");
    assert!(!term_mentions_global(&Term::var("fix#f"), "fix#f"));
}

// ==========================================================================
// D5 / D9 / D10 — the pre-pass
// ==========================================================================

#[test]
fn prepass_refuses_each_inadmissible_node() {
    let ctx = Context::new();
    assert_eq!(
        proof_prepass(&ctx, &Term::Return(Box::new(Term::Unit))),
        Err(TypeError::ReturnInProof)
    );
    // 9.10.26o AC3: the kernel refuses a region in a proof term, nested too.
    assert_eq!(
        proof_prepass(
            &ctx,
            &Term::region(
                crate::terms::RegionTypes::value_only(crate::types::Type::Nat),
                Term::Unit
            )
        ),
        Err(TypeError::RegionInProof)
    );
    assert_eq!(
        proof_prepass(
            &ctx,
            &Term::pair(
                Term::Unit,
                Term::region(
                    crate::terms::RegionTypes::value_only(crate::types::Type::Nat),
                    Term::Unit
                )
            )
        ),
        Err(TypeError::RegionInProof)
    );
    // 13.10.26n D3: the evaluator's call frame is refused the same way.
    assert_eq!(
        proof_prepass(&ctx, &Term::pair(Term::Unit, Term::call_frame(Term::Unit))),
        Err(TypeError::CallFrameInProof)
    );
    assert_eq!(
        proof_prepass(&ctx, &Term::Sorry),
        Err(TypeError::SorryInProof)
    );
    assert_eq!(
        proof_prepass(&ctx, &Term::pair(Term::Unit, Term::Sorry)),
        Err(TypeError::SorryInProof)
    );
    assert_eq!(
        proof_prepass(&ctx, &Term::annot(Term::Unit, Type::Error)),
        Err(TypeError::PoisonInProof { ty: Type::Error })
    );
    let nested_eq = Type::arrow(Type::Nat, Type::eq(Type::Nat, Term::Sorry, Term::Zero));
    assert_eq!(
        proof_prepass(&ctx, &Term::lambda("p", nested_eq, Term::Unit)),
        Err(TypeError::SorryInProof)
    );
    assert_eq!(
        proof_prepass(&ctx.with_term("h", Type::Error), &Term::Unit),
        Err(TypeError::PoisonInProof { ty: Type::Error })
    );
    assert_eq!(
        proof_prepass(&ctx, &Term::pair(Term::Unit, Term::Zero)),
        Ok(())
    );
}
