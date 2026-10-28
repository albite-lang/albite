//! ADR 29.9.26c AC 3: the reducer, the comparison and the renaming, rule by
//! rule. Pure: every test builds its terms and global map directly.

use std::collections::HashMap;

use crate::eval::EvalStopped;
use crate::{Term, Type};

use super::rename::RESERVED;
use super::*;

fn var(name: &str) -> Term {
    Term::var(name)
}

/// WHNF under no globals and a generous budget.
pub(super) fn head(term: Term) -> Term {
    whnf(&term, &HashMap::new(), &mut Fuel::new(10_000)).expect("within budget")
}

pub(super) fn same(left: Term, right: Term) -> Convertible {
    let (left, right) = rename_apart(&left, &right);
    convertible(&left, &right, &HashMap::new(), 10_000)
}

fn nat_sum() -> Type {
    Type::sum(Type::Nat, Type::Nat)
}

fn list_mu() -> Type {
    Type::mu("α_L", Type::sum(Type::Unit, Type::TyVar("α_L".into())))
}

// 29.9.26c AC3: each D3 rule fires on a constructor head with an OPEN payload.
#[test]
fn eliminations_fire_on_a_constructor_head_whatever_its_payload() {
    let pair = Term::pair(var("x"), var("y"));
    assert_eq!(head(Term::fst(pair.clone())), var("x"));
    assert_eq!(head(Term::snd(pair)), var("y"));
    assert_eq!(
        head(Term::unfold(list_mu(), Term::fold(list_mu(), var("x")))),
        var("x")
    );
    let case = |scrutinee| Term::case(scrutinee, "a", var("a"), "b", Term::Zero);
    assert_eq!(head(case(Term::inl(nat_sum(), var("x")))), var("x"));
    assert_eq!(head(case(Term::inr(nat_sum(), var("x")))), Term::Zero);
    assert_eq!(
        head(Term::if_then_else(Term::True, var("x"), var("y"))),
        var("x")
    );
    assert_eq!(
        head(Term::if_then_else(Term::False, var("x"), var("y"))),
        var("y")
    );
    let arms = vec![
        (0, "p".to_string(), Box::new(var("p"))),
        (1, "q".to_string(), Box::new(Term::Zero)),
    ];
    let construct = Term::adt_construct(Type::Nat, 0, var("x"));
    assert_eq!(head(Term::adt_match(construct, arms)), var("x"));
    let refl = Term::refl(Type::Nat, var("x"));
    assert_eq!(
        head(Term::subst(Type::Nat, Type::Nat, refl, var("p"))),
        var("p")
    );
    let type_abstraction = Term::ty_abs("T", var("x"));
    assert_eq!(head(Term::ty_app(type_abstraction, Type::Nat)), var("x"));
    assert_eq!(head(Term::annot(var("x"), Type::Nat)), var("x"));
}

// 29.9.26c AC3: β and let are by name — an open argument is not evaluated first.
#[test]
fn beta_and_let_substitute_the_argument_unevaluated() {
    let id = Term::lambda("a", Type::Nat, var("a"));
    let stuck_arg = Term::nat_sub(var("x"), var("y"));
    assert_eq!(head(Term::app(id, stuck_arg.clone())), stuck_arg);
    let let_in = Term::let_in("a", Type::Nat, stuck_arg.clone(), Term::succ(var("a")));
    assert_eq!(head(let_in), Term::succ(stuck_arg));
}

// 29.9.26c AC3: the same elimination over a variable is neutral, not a stop.
#[test]
fn an_elimination_of_a_variable_is_neutral() {
    for neutral in [
        Term::fst(var("x")),
        Term::unfold(list_mu(), var("x")),
        Term::case(var("x"), "a", var("a"), "b", var("b")),
        Term::if_then_else(var("x"), Term::Zero, Term::Zero),
        Term::natrec(Type::Nat, Term::Zero, var("s"), var("x")),
        Term::subst(Type::Nat, Type::Nat, var("e"), var("p")),
        Term::app(var("f"), Term::Zero),
    ] {
        assert_eq!(head(neutral.clone()), neutral);
    }
}

// 29.9.26c AC3: natrec/natind fire on each numeral spelling.
#[test]
fn recursors_fire_on_zero_succ_and_literals() {
    let rec = |n| Term::natrec(Type::Nat, var("z"), var("s"), n);
    assert_eq!(head(rec(Term::Zero)), var("z"));
    assert_eq!(head(rec(Term::NatLit(0))), var("z"));
    let step = |pred: Term| Term::app(Term::app(var("s"), pred.clone()), rec(pred));
    assert_eq!(head(rec(Term::succ(var("m")))), step(var("m")));
    assert_eq!(head(rec(Term::NatLit(3))), step(Term::NatLit(2)));
    let induction = Term::natind("k", Type::Nat, var("z"), var("s"), Term::Zero);
    assert_eq!(head(induction), var("z"));
}

// 29.9.26c AC3: D4, each arm of `+`.
#[test]
fn addition_recurses_on_its_second_operand() {
    let k = || var("k");
    assert_eq!(head(Term::nat_add(k(), Term::Zero)), k());
    assert_eq!(head(Term::nat_add(k(), Term::NatLit(0))), k());
    assert_eq!(
        head(Term::nat_add(k(), Term::succ(var("m")))),
        Term::succ(Term::nat_add(k(), var("m")))
    );
    assert_eq!(
        head(Term::nat_add(k(), Term::NatLit(3))),
        Term::succ(Term::nat_add(k(), Term::NatLit(2)))
    );
    assert_eq!(
        head(Term::nat_add(Term::NatLit(2), Term::NatLit(3))),
        Term::NatLit(5)
    );
    let left_zero = Term::nat_add(Term::Zero, k());
    assert_eq!(head(left_zero.clone()), left_zero, "0 + k needs induction");
    assert!(matches!(same(left_zero, k()), Convertible::No { .. }));
}

// 29.9.26c AC3: the sibling's exact Core, and `k + 1 ≡ succ(k)`.
#[test]
fn the_sibling_step_case_shapes_are_convertible() {
    let k = || var("k");
    assert_eq!(
        same(
            Term::nat_add(Term::succ(k()), Term::Zero),
            Term::succ(Term::nat_add(k(), Term::Zero))
        ),
        Convertible::Yes
    );
    assert_eq!(
        same(Term::nat_add(k(), Term::succ(Term::Zero)), Term::succ(k())),
        Convertible::Yes
    );
    assert_eq!(
        same(Term::nat_add(k(), Term::NatLit(1)), Term::succ(k())),
        Convertible::Yes
    );
    assert!(matches!(
        same(Term::nat_add(k(), Term::NatLit(2)), Term::succ(k())),
        Convertible::No { .. }
    ));
}

// 29.9.26c AC3: D3's closed primitives compute beside an open argument.
#[test]
fn a_closed_primitive_computes_and_an_open_one_is_neutral() {
    assert_eq!(head(Term::nat_eq(Term::NatLit(0), Term::Zero)), Term::True);
    assert_eq!(head(Term::nat_eq(Term::NatLit(1), Term::Zero)), Term::False);
    let difference = head(Term::nat_sub(Term::nat(2), Term::NatLit(1)));
    assert_eq!(crate::eval::term_to_nat(&difference), Some(1));
    assert_eq!(
        head(Term::bool_not(Term::fst(Term::pair(Term::True, var("x"))))),
        Term::False
    );
    let open = Term::nat_eq(var("m"), Term::Zero);
    assert_eq!(head(open.clone()), open);
    let guarded = Term::if_then_else(Term::nat_eq(Term::Zero, Term::Zero), Term::Zero, var("n"));
    assert_eq!(head(guarded), Term::Zero);
}

// 29.9.26c AC3: an absent global is neutral; a `Return` body is not unfolded.
#[test]
fn globals_unfold_only_when_present_and_effect_free() {
    let two = Term::global("two");
    assert_eq!(head(two.clone()), two);
    let globals: HashMap<String, Term> = [
        ("plain".to_string(), Term::NatLit(3)),
        ("early".to_string(), Term::early_return(Term::NatLit(3))),
    ]
    .into();
    let unfoldable = unfoldable_globals(&globals);
    assert!(unfoldable.contains_key("plain"));
    assert!(
        !unfoldable.contains_key("early"),
        "D3: Return is WHNF-opaque"
    );
    let mut fuel = Fuel::new(100);
    assert_eq!(
        whnf(&Term::global("plain"), &unfoldable, &mut fuel),
        Ok(Term::NatLit(3))
    );
    let early = Term::global("early");
    assert_eq!(whnf(&early, &unfoldable, &mut fuel), Ok(early));
}

// 29.9.26c AC3: D2 — the §1.2 capture shape reduces to `5 + n`, never `10`.
#[test]
fn renaming_apart_prevents_capture() {
    // apply = λg. let n = 5 in g(n);  applied to λx. x + n (the caller's n).
    let apply = Term::lambda(
        "g",
        Type::arrow(Type::Nat, Type::Nat),
        Term::let_in(
            "n",
            Type::Nat,
            Term::NatLit(5),
            Term::app(var("g"), var("n")),
        ),
    );
    let call = Term::app(
        apply,
        Term::lambda("x", Type::Nat, Term::nat_add(var("x"), var("n"))),
    );
    let ten = Term::NatLit(10);
    let (left, right) = rename_apart(&call, &ten);
    assert_eq!(right, ten, "a closed side is untouched");
    assert!(left.free_vars().contains("?n"));
    let reduced = head(left);
    assert_eq!(reduced, Term::nat_add(Term::NatLit(5), var("?n")));
    assert_eq!(Renaming::display(&reduced), "(5 + n)");
    let Convertible::No { first_mismatch } = same(call, ten) else {
        panic!("5 + n is not 10");
    };
    assert!(!Renaming::display(&first_mismatch.0).contains('?'));
}

// 29.9.26c AC3: fresh binder variables never collide with a renamed one.
#[test]
fn fresh_binders_are_distinct_and_display_as_placeholders() {
    let mut binders = Renaming::default();
    let (first, second) = (binders.fresh(), binders.fresh());
    assert_ne!(first, second);
    assert!(first.starts_with(RESERVED));
    assert_eq!(Renaming::display(&var(&first)), "_0");
    assert_eq!(
        Renaming::display(&var("plain")),
        "plain",
        "unreserved names are left alone"
    );
}

// 29.9.26c AC3: D6 — one budget; a long numeral finishes without recursion.
#[test]
fn the_budget_is_shared_and_long_numerals_use_the_worklist() {
    // Ten `Succ`s to peel, two steps each: well over five.
    let slow = Term::nat_add(var("k"), Term::NatLit(10));
    let (left, right) = rename_apart(&slow, &Term::NatLit(10));
    assert_eq!(
        convertible(&left, &right, &HashMap::new(), 5),
        Convertible::Stopped(EvalStopped::StepLimit { limit: 5 })
    );
    let big = Term::NatLit(100_000);
    let (left, right) = rename_apart(&big, &Term::nat_add(var("k"), Term::NatLit(100_000)));
    assert!(matches!(
        convertible(&left, &right, &HashMap::new(), 1_000_000),
        Convertible::No { .. }
    ));
    assert_eq!(
        same(Term::NatLit(100_000), Term::NatLit(100_000)),
        Convertible::Yes
    );
}

// 29.9.26c AC3: λ against λ under one fresh variable; there is no η.
#[test]
fn lambdas_compare_under_a_fresh_variable_without_eta() {
    let left = Term::lambda("x", Type::Nat, Term::nat_add(var("x"), Term::Zero));
    let right = Term::lambda("y", Type::Nat, var("y"));
    assert_eq!(same(left, right), Convertible::Yes);
    let eta = Term::lambda("x", Type::Nat, Term::app(var("f"), var("x")));
    assert!(matches!(same(eta, var("f")), Convertible::No { .. }));
    let other_type = Term::lambda("y", Type::Bool, var("y"));
    let nat_id = Term::lambda("x", Type::Nat, var("x"));
    assert!(matches!(same(nat_id, other_type), Convertible::No { .. }));
}

// 29.9.26c AC3: `Fix` is neutral and compared structurally, never unfolded.
#[test]
fn fix_is_neutral() {
    let fix = |name: &str| Term::fix(name, Type::Nat, Term::succ(var(name)));
    assert_eq!(head(fix("f")), fix("f"));
    assert_eq!(
        same(fix("f"), fix("g")),
        Convertible::Yes,
        "alpha-equivalent"
    );
    assert!(matches!(
        same(fix("f"), Term::succ(fix("f"))),
        Convertible::No { .. }
    ));
}

// 29.9.26c AC3: neutral eliminations compare their arguments pairwise.
#[test]
fn neutral_eliminations_compare_argument_by_argument() {
    let case = |zero: Term| Term::case(var("s"), "a", zero, "b", var("b"));
    assert_eq!(
        same(
            case(Term::nat_add(Term::NatLit(1), Term::Zero)),
            case(Term::NatLit(1))
        ),
        Convertible::Yes
    );
    assert!(matches!(
        same(case(Term::Zero), case(Term::NatLit(1))),
        Convertible::No { .. }
    ));
    let rec = |n| Term::natrec(Type::Nat, Term::Zero, var("s"), n);
    assert_eq!(
        same(rec(Term::nat_add(var("m"), Term::Zero)), rec(var("m"))),
        Convertible::Yes
    );
    let arm = |body: Term| vec![(0, "p".to_string(), Box::new(body))];
    assert_eq!(
        same(
            Term::adt_match(var("s"), arm(var("p"))),
            Term::adt_match(var("s"), arm(var("p")))
        ),
        Convertible::Yes
    );
    assert!(matches!(
        same(Term::global("f"), Term::global("g")),
        Convertible::No { .. }
    ));
}

// 29.9.26c AC3: D5 — an effectful side is never WHNF-compared.
#[test]
fn reference_cells_are_refused_before_by_name_reduction() {
    let cell = || Term::ref_new(Term::Zero);
    let written = Term::let_in(
        "r",
        Type::Nat,
        cell(),
        Term::let_in(
            "_",
            Type::Unit,
            Term::ref_set(var("r"), Term::NatLit(1)),
            Term::ref_get(var("r")),
        ),
    );
    let read = Term::let_in("r", Type::Nat, cell(), Term::ref_get(var("r")));
    assert!(!unfoldable_by_name(&written));
    assert!(!unfoldable_by_name(&read));
    assert!(!unfoldable_by_name(&Term::extern_call(
        "tg_anything",
        vec![]
    )));
    assert!(unfoldable_by_name(&Term::nat_add(var("k"), Term::Zero)));
    // The hazard the guard exists for: by name, both sides reach `!(ref 0)`.
    assert_eq!(same(written, read), Convertible::Yes);
    // The discharge half — CBV's 1 ≠ 0 survives — stays in the bootstrap as
    // `a_reference_cell_obligation_keeps_its_cbv_verdict` (ADR 30.9.26g D6).
}

/// 29.9.26d: two `natind` heads agree only with the same binder and motive.
#[test]
fn natind_heads_agree_only_with_the_same_binder_and_motive() {
    let natind = |binder: &str, motive: Type| {
        Term::natind(binder, motive, Term::Zero, Term::Zero, Term::var("n"))
    };
    let motive = Type::arrow(Type::Nat, Type::Nat);
    let head = natind("k", motive.clone());
    assert!(compare::same_head(&head, &natind("k", motive.clone())));
    assert!(!compare::same_head(&head, &natind("j", motive)));
    assert!(!compare::same_head(
        &head,
        &natind("k", Type::arrow(Type::Nat, Type::Bool))
    ));
}
