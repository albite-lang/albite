//! `cases` (ADR 3.10.26f AC 2): the split's shape over hand-built Core here,
//! and the elaboration half and D4's refusals in `elaboration.rs`.
//!
//! The kernel's own `Cᵢ` instance is private to `albite_core`, so "the
//! goal converts to it" is asserted the one way the kernel exposes:
//! `check_proof` of the annotated split, each arm proved by `refl` at the
//! goal `split.rs` built — accepted exactly when every arm goal is
//! `types_equal_in` (proof mode) to the rule's instance.

mod elaboration;
mod kernel_refusal;
mod pair;

use albite_core::typecheck::check_proof;
use albite_core::{Context, Term, Type};

use super::split::{
    arm_body, arm_goal, constructor_value, field_types, split_shape, split_term, Former, SplitArm,
    SplitShape,
};

/// `Opt = None | Some(Nat)`, a binary sum.
pub(super) fn opt() -> Type {
    Type::sum(Type::Unit, Type::Nat)
}

/// `Lst = μα_Lst. Unit + (Nat × α_Lst)`, as the elaborator encodes it.
fn lst() -> Type {
    Type::mu(
        "α_Lst",
        Type::sum(
            Type::Unit,
            Type::product(Type::Nat, Type::TyVar("α_Lst".to_string())),
        ),
    )
}

/// `Tri = Lo | Mid(Nat) | Hi`, a flat ADT.
fn tri() -> Type {
    Type::adt(
        "Tri",
        vec![],
        vec![
            ("Lo".to_string(), Type::Unit),
            ("Mid".to_string(), Type::Nat),
            ("Hi".to_string(), Type::Unit),
        ],
    )
}

pub(super) fn shape_of(ty: &Type) -> SplitShape {
    split_shape(ty).expect("a splittable type")
}

fn names(fields: &[&str]) -> Vec<String> {
    fields.iter().map(|name| (*name).to_string()).collect()
}

/// `Eq τ v v`: every arm's instance is `Eq τ Cᵢ(z) Cᵢ(z)`, which `refl`
/// proves at exactly the constructor value the arm goal spells.
fn reflexive(ty: &Type, variable: &str) -> Type {
    Type::eq(ty.clone(), Term::var(variable), Term::var(variable))
}

/// The annotated split of `goal` over `variable : ty`, each arm closed by
/// `refl` at the constructor value `split.rs` spells for it.
fn refl_split(variable: &str, ty: &Type, goal: &Type, arms: &[(usize, &[(&str, Type)])]) -> Term {
    let shape = shape_of(ty);
    let split_arms = arms
        .iter()
        .map(|(index, fields)| {
            let fields: Vec<(String, Type)> = fields
                .iter()
                .map(|(n, t)| ((*n).to_string(), t.clone()))
                .collect();
            let field_names: Vec<String> = fields.iter().map(|(n, _)| n.clone()).collect();
            let binder = format!("%b{index}");
            let proof = Term::refl(ty.clone(), constructor_value(&shape, *index, &field_names));
            let hole = "%proof";
            let body = graft_hole(arm_body(&binder, &fields, hole), hole, &proof);
            SplitArm {
                index: *index,
                binder,
                body,
            }
        })
        .collect();
    split_term(variable, &shape, split_arms, goal).expect("an arm per sum index")
}

/// `body` with its one hole `Var(hole)` replaced, under the λ-chain.
fn graft_hole(body: Term, hole: &str, proof: &Term) -> Term {
    match body {
        Term::Var(name) if name == hole => proof.clone(),
        Term::Lambda(binder, ty, inner) => {
            Term::lambda(binder, ty, graft_hole(*inner, hole, proof))
        }
        Term::App(head, arg) => Term::app(graft_hole(*head, hole, proof), *arg),
        other => other,
    }
}

fn proof_ctx(variable: &str, ty: &Type) -> Context {
    Context::new().with_term(variable, ty.clone()).for_proof()
}

// ADR 3.10.26f AC2: P0.2's spellings, from `albite info def`, and the
// kernel's instance accepting each arm goal.
#[test]
fn each_arm_goal_spells_the_constructor_as_the_elaborator_does() {
    let opt_shape = shape_of(&opt());
    assert_eq!(
        constructor_value(&opt_shape, 0, &[]),
        Term::inl(opt(), Term::Unit)
    );
    assert_eq!(
        constructor_value(&opt_shape, 1, &names(&["x"])),
        Term::inr(opt(), Term::var("x"))
    );
    let lst_shape = shape_of(&lst());
    let lst_body = Type::sum(Type::Unit, Type::product(Type::Nat, lst()));
    assert_eq!(
        constructor_value(&lst_shape, 0, &[]),
        Term::fold(lst(), Term::inl(lst_body.clone(), Term::Unit))
    );
    assert_eq!(
        constructor_value(&lst_shape, 1, &names(&["h", "t"])),
        Term::fold(
            lst(),
            Term::inr(lst_body, Term::pair(Term::var("h"), Term::var("t")))
        )
    );
    let tri_shape = shape_of(&tri());
    assert_eq!(
        constructor_value(&tri_shape, 1, &names(&["n"])),
        Term::adt_construct(tri(), 1, Term::var("n"))
    );
    assert_eq!(
        constructor_value(&tri_shape, 2, &[]),
        Term::adt_construct(tri(), 2, Term::Unit)
    );
    assert_eq!(
        arm_goal("o", &reflexive(&opt(), "o"), &opt_shape, 1, &names(&["x"])),
        Type::eq(
            opt(),
            Term::inr(opt(), Term::var("x")),
            Term::inr(opt(), Term::var("x"))
        )
    );

    // The kernel's rule accepts each split, so each arm goal converts to its
    // instance.
    let some_fields = [("x", Type::Nat)];
    let cons_fields = [("h", Type::Nat), ("t", lst())];
    let mid_fields = [("n", Type::Nat)];
    let cases: [(&str, Type, Vec<(usize, &[(&str, Type)])>); 3] = [
        ("o", opt(), vec![(0, &[]), (1, &some_fields)]),
        ("l", lst(), vec![(0, &[]), (1, &cons_fields)]),
        ("t", tri(), vec![(0, &[]), (1, &mid_fields), (2, &[])]),
    ];
    for (variable, ty, arms) in cases {
        let goal = reflexive(&ty, variable);
        let split = refl_split(variable, &ty, &goal, &arms);
        assert_eq!(
            check_proof(&proof_ctx(variable, &ty), &split, &goal),
            Ok(()),
            "{variable}: {split}"
        );
    }
}

// ADR 3.10.26f AC2: a sum's two arms are placed by index whatever their
// order, and an ADT's split names each index once.
#[test]
fn every_index_appears_once() {
    let arm = |index: usize| SplitArm {
        index,
        binder: format!("b{index}"),
        body: Term::var(format!("p{index}")),
    };
    let goal = reflexive(&opt(), "o");
    let sum = split_term("o", &shape_of(&opt()), vec![arm(1), arm(0)], &goal).unwrap();
    let Term::Annot(inner, annotation) = &sum else {
        panic!("expected an annotation, got {sum}");
    };
    assert_eq!(*annotation, goal);
    assert_eq!(
        **inner,
        Term::case(Term::var("o"), "b0", Term::var("p0"), "b1", Term::var("p1"))
    );
    assert_eq!(
        split_term("o", &shape_of(&opt()), vec![arm(1)], &goal),
        None
    );

    let tri_goal = reflexive(&tri(), "t");
    let flat = split_term(
        "t",
        &shape_of(&tri()),
        vec![arm(2), arm(0), arm(1)],
        &tri_goal,
    )
    .unwrap();
    let Term::Annot(inner, _) = &flat else {
        panic!("expected an annotation, got {flat}");
    };
    let Term::AdtMatch(scrutinee, arms) = &**inner else {
        panic!("expected an adt_match, got {inner}");
    };
    assert_eq!(**scrutinee, Term::var("t"));
    let mut indices: Vec<usize> = arms.iter().map(|(index, _, _)| *index).collect();
    indices.sort_unstable();
    assert_eq!(indices, [0, 1, 2]);

    // A μ is split through `unfold`, at the declared μ.
    let unfolded = split_term(
        "l",
        &shape_of(&lst()),
        vec![arm(0), arm(1)],
        &reflexive(&lst(), "l"),
    );
    let Some(Term::Annot(inner, _)) = unfolded else {
        panic!("expected an annotation");
    };
    let Term::Case(scrutinee, ..) = *inner else {
        panic!("expected a case");
    };
    assert_eq!(*scrutinee, Term::unfold(lst(), Term::var("l")));
}

// ADR 3.10.26f AC2: the shapes that do not split.
#[test]
fn only_sums_and_flat_adts_split() {
    assert_eq!(shape_of(&opt()).former, Former::Sum);
    assert_eq!(shape_of(&opt()).mu, None);
    assert_eq!(shape_of(&tri()).former, Former::Adt);
    assert_eq!(
        shape_of(&tri()).payloads,
        [Type::Unit, Type::Nat, Type::Unit]
    );
    assert_eq!(shape_of(&lst()).mu, Some(lst()));
    for ty in [
        Type::Nat,
        Type::Bool,
        Type::product(Type::Nat, Type::Nat),
        Type::TyVar("Pt".into()),
    ] {
        assert_eq!(split_shape(&ty), None, "{ty}");
    }
}

// ADR 3.10.26f AC2: the arm's binder is synthesised, so it is never free in
// the goal it splits, nor a name the user can write.
#[test]
fn arm_binders_are_fresh_in_the_goal() {
    let source = "type Opt = None | Some(Nat)
theorem opt_refl(o: Opt) -> Eq<Opt, o, o> by { cases o { None => refl, Some(_) => refl } }";
    let defs = crate::elaborate::tests::elab_ok(source);
    let def = defs
        .iter()
        .find(|d| d.name == "opt_refl")
        .expect("the theorem");
    let Term::Lambda(_, _, body) = &def.term.term else {
        panic!("expected the parameter's λ, got {}", def.term.term);
    };
    let Term::Annot(split, goal) = &**body else {
        panic!("expected the annotated split, got {body}");
    };
    let Term::Case(_, left, _, right, right_body) = &**split else {
        panic!("expected a case, got {split}");
    };
    let free = goal.free_term_vars();
    for binder in [left, right] {
        assert!(binder.starts_with('%'), "{binder}");
        assert!(!free.contains(binder), "{binder} is free in {goal}");
    }
    assert_ne!(left, right);
    // `_` is a synthesised field name too.
    let Term::App(function, _) = &**right_body else {
        panic!("expected the one-field chain, got {right_body}");
    };
    let Term::Lambda(field, _, _) = &**function else {
        panic!("expected the field's λ, got {function}");
    };
    assert!(field.starts_with('%') && !free.contains(field), "{field}");
}

// ADR 3.10.26f AC2: the chain carries the declared field count, which the
// encoding cannot — a record payload is one field, a two-field one two.
#[test]
fn the_chain_length_is_the_declared_field_count() {
    let pair = Type::product(Type::Nat, Type::Nat);
    assert_eq!(field_types(&pair, 1), Some(vec![pair.clone()]));
    assert_eq!(field_types(&pair, 2), Some(vec![Type::Nat, Type::Nat]));
    assert_eq!(field_types(&pair, 3), None);
    assert_eq!(field_types(&Type::Unit, 0), Some(vec![]));
    assert_eq!(field_types(&Type::Nat, 0), None);

    let fields = [
        ("a".to_string(), Type::Nat),
        ("b".to_string(), Type::Nat),
        ("c".to_string(), Type::Nat),
    ];
    let body = arm_body("p", &fields, "%goal7");
    let lambdas = fields
        .iter()
        .rev()
        .fold(Term::var("%goal7"), |inner, (name, ty)| {
            Term::lambda(name, ty.clone(), inner)
        });
    let expected = Term::app(
        Term::app(
            Term::app(lambdas, Term::fst(Term::var("p"))),
            Term::fst(Term::snd(Term::var("p"))),
        ),
        Term::snd(Term::snd(Term::var("p"))),
    );
    assert_eq!(body, expected);
    assert_eq!(arm_body("p", &[], "%goal7"), Term::var("%goal7"));
    // The test builder fills only the hole, never another variable.
    assert_eq!(
        graft_hole(Term::var("p"), "%goal7", &Term::Unit),
        Term::var("p")
    );
    assert_eq!(
        graft_hole(Term::var("%goal7"), "%goal7", &Term::Unit),
        Term::Unit
    );
    assert_eq!(
        arm_body("p", &fields[..1], "%goal7"),
        Term::app(
            Term::lambda("a", Type::Nat, Term::var("%goal7")),
            Term::var("p")
        )
    );
}
