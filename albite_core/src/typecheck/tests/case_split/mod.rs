//! The kernel's dependent case split (ADR 3.10.26o): AC 1's positives, the
//! open-reduct invariant and the nested cost, and AC 3's hand-built split.
//! The must-fail twins are in `twins.rs`, AC 2's unchanged verdicts in
//! `verdicts.rs`. Shared builders below.

use super::*;
use crate::context::{Context, GlobalAdmission};
use crate::conversion::{convertible, Convertible, CONVERSION_STEP_BUDGET};
use crate::terms::Term;
use crate::typecheck::rules_ext::{is_case_split, type_of_dependent_case, SPLITS_CHECKED};
use crate::types::Type;

mod twins;
mod verdicts;

/// `Opt = None | Some(Nat)`, a binary sum.
fn opt() -> Type {
    Type::sum(Type::Unit, Type::Nat)
}

fn none() -> Term {
    Term::inl(opt(), Term::Unit)
}

fn some(payload: Term) -> Term {
    Term::inr(opt(), payload)
}

/// `case o of inl u => None | inr x => Some(x)`: the identity, written as a
/// split so each constructor instance reduces by ι alone.
fn rebuild_opt(scrutinee: Term) -> Term {
    Term::case(scrutinee, "u", none(), "x", some(Term::var("x")))
}

/// `Eq Opt (rebuild o) o`.
fn rebuild_goal(variable: &str) -> Type {
    Type::eq(opt(), rebuild_opt(Term::var(variable)), Term::var(variable))
}

/// `(λz₁:A₁. … λzₖ:Aₖ. proof) π₁(b) … πₖ(b)` over `binder`.
fn chain(binder: &str, fields: &[(&str, Type)], proof: Term) -> Term {
    let count = fields.len();
    let function = fields.iter().rev().fold(proof, |body, (name, ty)| {
        Term::lambda(*name, ty.clone(), body)
    });
    (0..count).fold(function, |applied, index| {
        Term::app(applied, projection(binder, index, count))
    })
}

/// `πᵢ(b)` of a right-nested `count`-tuple.
fn projection(binder: &str, index: usize, count: usize) -> Term {
    let rest = (0..index).fold(Term::var(binder), |term, _| Term::snd(term));
    if index + 1 < count {
        Term::fst(rest)
    } else {
        rest
    }
}

/// `case o of inl u => refl None | inr b => (λx:Nat. refl (Some x)) b`.
fn opt_split(scrutinee: &str) -> Term {
    Term::case(
        Term::var(scrutinee),
        "u",
        Term::refl(opt(), none()),
        "b",
        chain(
            "b",
            &[("x", Type::Nat)],
            Term::refl(opt(), some(Term::var("x"))),
        ),
    )
}

fn proof_over(bindings: &[(&str, Type)]) -> Context {
    bindings
        .iter()
        .fold(Context::new(), |ctx, (name, ty)| {
            ctx.with_term(*name, ty.clone())
        })
        .for_proof()
}

/// `Lst = μL. Unit + (Nat × L)`.
fn lst() -> Type {
    Type::mu(
        "L",
        Type::sum(
            Type::Unit,
            Type::product(Type::Nat, Type::TyVar("L".to_string())),
        ),
    )
}

/// `Lst`'s unfolded body, `Unit + (Nat × Lst)`.
fn lst_body() -> Type {
    Type::sum(Type::Unit, Type::product(Type::Nat, lst()))
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

/// `Eq τ v v`.
fn reflexive_goal(ty: Type, variable: &str) -> Type {
    Type::eq(ty, Term::var(variable), Term::var(variable))
}

fn splits_checked() -> usize {
    SPLITS_CHECKED.with(std::cell::Cell::get)
}

/// 3.10.26o AC1: a sum splits on a variable, k = 0 (`None`) and k = 1 (`Some`).
#[test]
fn a_sum_splits_on_a_variable() {
    let ctx = proof_over(&[("o", opt())]);
    let goal = rebuild_goal("o");
    assert_eq!(
        type_of_dependent_case(&ctx, &opt_split("o"), &goal),
        Ok(goal.clone())
    );
    assert_eq!(
        type_of(&ctx, &Term::annot(opt_split("o"), goal.clone())),
        Ok(goal)
    );
}

/// 3.10.26o AC1: the elaborator's source spans are transparent — around the
/// split, its scrutinee, and the binder a projection reads.
#[test]
fn a_spanned_split_still_splits() {
    let span = |term| Term::spanned(term, crate::terms::TermSpan::new(0, 1));
    let split = span(Term::case(
        span(Term::var("o")),
        "u",
        Term::refl(opt(), none()),
        "b",
        Term::app(
            Term::lambda("x", Type::Nat, Term::refl(opt(), some(Term::var("x")))),
            span(Term::var("b")),
        ),
    ));
    let ctx = proof_over(&[("o", opt())]);
    let goal = rebuild_goal("o");
    assert!(is_case_split(&split));
    assert_eq!(type_of_dependent_case(&ctx, &split, &goal), Ok(goal));
}

/// 3.10.26o AC1: an application whose head is no λ is an arm with no fields
/// and that application as its proof — here a hypothesis applied to `()`.
#[test]
fn an_arm_proved_by_applying_a_hypothesis_binds_no_fields() {
    let none_case = Type::arrow(Type::Unit, Type::eq(opt(), rebuild_opt(none()), none()));
    let split = Term::case(
        Term::var("o"),
        "u",
        Term::app(Term::var("f"), Term::Unit),
        "b",
        chain(
            "b",
            &[("x", Type::Nat)],
            Term::refl(opt(), some(Term::var("x"))),
        ),
    );
    let ctx = proof_over(&[("o", opt()), ("f", none_case)]);
    let goal = rebuild_goal("o");
    assert_eq!(type_of_dependent_case(&ctx, &split, &goal), Ok(goal));
}

/// 3.10.26o AC1: an unfolded μ splits, each instance folded; k = 0 and k = 2.
#[test]
fn an_unfolded_mu_splits_with_fold() {
    let rebuild = |scrutinee: Term| {
        Term::case(
            Term::unfold(lst(), scrutinee),
            "u",
            Term::fold(lst(), Term::inl(lst_body(), Term::Unit)),
            "p",
            Term::fold(lst(), Term::inr(lst_body(), Term::var("p"))),
        )
    };
    let goal = Type::eq(lst(), rebuild(Term::var("l")), Term::var("l"));
    let cons = Term::fold(
        lst(),
        Term::inr(lst_body(), Term::pair(Term::var("h"), Term::var("t"))),
    );
    let split = Term::case(
        Term::unfold(lst(), Term::var("l")),
        "u",
        Term::refl(lst(), Term::fold(lst(), Term::inl(lst_body(), Term::Unit))),
        "b",
        chain(
            "b",
            &[("h", Type::Nat), ("t", lst())],
            Term::refl(lst(), cons),
        ),
    );
    let ctx = proof_over(&[("l", lst())]);
    assert_eq!(
        type_of_dependent_case(&ctx, &split, &goal),
        Ok(goal.clone())
    );
    // The old rule refuses: the arms prove two different instances.
    assert!(type_of(&ctx, &split).is_err());
    assert_eq!(type_of(&ctx, &Term::annot(split, goal.clone())), Ok(goal));
}

/// 3.10.26o AC1: a flat ADT splits by index, whatever order the arms are in.
#[test]
fn a_flat_adt_splits_by_index() {
    let construct = |index, payload| Term::adt_construct(tri(), index, payload);
    let split = Term::adt_match(
        Term::var("t"),
        vec![
            Term::adt_arm(2, "c", Term::refl(tri(), construct(2, Term::Unit))),
            Term::adt_arm(0, "a", Term::refl(tri(), construct(0, Term::Unit))),
            Term::adt_arm(
                1,
                "b",
                chain(
                    "b",
                    &[("n", Type::Nat)],
                    Term::refl(tri(), construct(1, Term::var("n"))),
                ),
            ),
        ],
    );
    let ctx = proof_over(&[("t", tri())]);
    let goal = reflexive_goal(tri(), "t");
    assert_eq!(type_of_dependent_case(&ctx, &split, &goal), Ok(goal));
}

/// 3.10.26o AC1: a one-field constructor whose field is a record binds the
/// whole product once — the chain, not the encoding, says how many fields.
#[test]
fn a_product_payload_binds_one_name_when_the_chain_is_one() {
    let point = Type::product(Type::Nat, Type::Nat);
    let shape = Type::adt("Shape", vec![], vec![("Dot".to_string(), point.clone())]);
    let dot = Term::adt_construct(shape.clone(), 0, Term::var("p"));
    let split = Term::adt_match(
        Term::var("s"),
        vec![Term::adt_arm(
            0,
            "b",
            chain("b", &[("p", point)], Term::refl(shape.clone(), dot)),
        )],
    );
    let ctx = proof_over(&[("s", shape.clone())]);
    let goal = reflexive_goal(shape, "s");
    assert_eq!(type_of_dependent_case(&ctx, &split, &goal), Ok(goal));
}

/// 3.10.26o AC1 (D3): the kernel never re-types a reduct. A split applied to
/// an open `inl w` checks, while its β-reduct does not type — the unit-η gap
/// the rule asserts — and conversion decides the goal without typing.
#[test]
fn an_open_reduct_is_never_retyped() {
    let lambda = Term::lambda("o", opt(), Term::annot(opt_split("o"), rebuild_goal("o")));
    let open = Term::inl(opt(), Term::var("w"));
    let ctx = Context::new().with_term("w", Type::Unit);
    let goal = Type::eq(opt(), rebuild_opt(open.clone()), open.clone());
    assert_eq!(
        check_proof(&ctx, &Term::app(lambda, open.clone()), &goal),
        Ok(())
    );

    let reduct = Term::annot(
        Term::case(
            open.clone(),
            "u",
            Term::refl(opt(), none()),
            "b",
            chain(
                "b",
                &[("x", Type::Nat)],
                Term::refl(opt(), some(Term::var("x"))),
            ),
        ),
        goal,
    );
    assert!(type_of_proof(&ctx, &reduct).is_err());

    let globals = std::collections::HashMap::new();
    let decide =
        |left: &Term, right: &Term| convertible(left, right, &globals, CONVERSION_STEP_BUDGET);
    assert_eq!(
        decide(&rebuild_opt(open.clone()), &none()),
        Convertible::Yes
    );
    assert_ne!(decide(&rebuild_opt(open.clone()), &open), Convertible::Yes);
}

/// The variables `o1 … o6`, each an `Opt`.
fn depth_variables(depth: usize) -> Vec<String> {
    (1..=depth).map(|level| format!("o{level}")).collect()
}

/// Right-nested `Opt × (Opt × …)` and its tuples, `depth` long.
fn opt_tuple_type(depth: usize) -> Type {
    (1..depth).fold(opt(), |rest, _| Type::product(opt(), rest))
}

fn right_tuple(items: Vec<Term>) -> Term {
    let mut items = items.into_iter().rev();
    let last = items.next().expect("a tuple has an item");
    items.fold(last, |rest, item| Term::pair(item, rest))
}

/// Split on each variable in turn: `goal` is the current instance, `values`
/// the constructor each variable split so far became.
fn nested_split(variables: &[String], goal: &Type, values: &[Term]) -> Term {
    let level = values.len();
    if level == variables.len() {
        return Term::refl(opt_tuple_type(level), right_tuple(values.to_vec()));
    }
    let variable = &variables[level];
    let field = format!("x{level}");
    let arm = |value: Term| {
        let instance = goal.substitute_term(variable, &value);
        let mut next = values.to_vec();
        next.push(value);
        Term::annot(nested_split(variables, &instance, &next), instance)
    };
    Term::case(
        Term::var(variable),
        format!("u{level}"),
        arm(none()),
        format!("b{level}"),
        chain(
            &format!("b{level}"),
            &[(&field, Type::Nat)],
            arm(some(Term::var(&field))),
        ),
    )
}

/// 3.10.26o AC1 (D5): six nested splits — 63 split nodes — are each checked
/// a bounded number of times, never once per path through the tree.
#[test]
fn a_depth_six_split_checks_within_budget() {
    let depth = 6;
    let variables = depth_variables(depth);
    let rebuilt = variables
        .iter()
        .map(|v| rebuild_opt(Term::var(v)))
        .collect();
    let originals = variables.iter().map(|v| Term::var(v)).collect();
    let goal = Type::eq(
        opt_tuple_type(depth),
        right_tuple(rebuilt),
        right_tuple(originals),
    );
    let bindings: Vec<(&str, Type)> = variables.iter().map(|v| (v.as_str(), opt())).collect();
    let ctx = proof_over(&bindings);

    let proof = Term::annot(nested_split(&variables, &goal, &[]), goal.clone());
    let before = splits_checked();
    assert_eq!(type_of(&ctx, &proof), Ok(goal));
    let checked = splits_checked() - before;
    let split_nodes = (1 << depth) - 1;
    // Today's rule first measured 1365 (P0.4): each level re-typed its subtree.
    assert_eq!(checked, split_nodes, "each split node is checked once");
}

/// 3.10.26o AC3: an η-free split passes `check_proof` with no global and no
/// δ: the statement is written with the split, so each arm closes by ι.
#[test]
fn a_hand_built_split_passes_check_proof() {
    let proof = Term::lambda("o", opt(), Term::annot(opt_split("o"), rebuild_goal("o")));
    let statement = Type::pi("o", opt(), rebuild_goal("o"));
    assert_eq!(check_proof(&Context::new(), &proof, &statement), Ok(()));
}

/// 3.10.26o AC3: the same statement, proved without the split, is refused.
#[test]
fn the_same_statement_without_the_split_is_refused() {
    let proof = Term::lambda("o", opt(), Term::refl(opt(), Term::var("o")));
    let statement = Type::pi("o", opt(), rebuild_goal("o"));
    assert!(check_proof(&Context::new(), &proof, &statement).is_err());
}

/// A context binding `g : Opt` as an admitted global, for the twins.
fn with_admitted_global(name: &str) -> Context {
    Context::new()
        .with_global(name, opt(), GlobalAdmission::Admitted)
        .for_proof()
}
