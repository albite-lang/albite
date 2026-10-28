//! ADR 2.10.26l AC 1 and AC 3: the kernel unfolds a body it admitted itself —
//! `Total`, effect-free, typed by `check_proof` — and nothing else. The
//! must-fail twins (D6) come first: each is a body the kernel must keep
//! neutral, so the proof that needs it is refused.

use std::collections::BTreeSet;

use super::*;
use crate::context::GlobalAdmission;
use crate::conversion::Unfoldings;
use crate::terms::termination::tests::fixtures::{
    length_body, list_fn, list_of, list_to_nat, list_type, match_list,
};
use crate::typecheck::{DeltaAdmission, DeltaCandidate};

/// One hand-built definition: its type, body, the report's `Total` bit and
/// its `#[decreasing]`.
struct Def {
    name: &'static str,
    ty: Type,
    body: Term,
    total: bool,
}

fn def(name: &'static str, ty: Type, body: Term) -> Def {
    Def {
        name,
        ty,
        body,
        total: true,
    }
}

impl Def {
    /// The same definition, `Partial` in the report (`#[partial]` or taint).
    fn partial(mut self) -> Self {
        self.total = false;
        self
    }
}

/// The context a theorem mentioning `roots` is checked in, as the bootstrap
/// builds it: each root bound with its report marker, then δ's admitted
/// bodies over the roots' mention closure.
fn bound(defs: &[Def], roots: &[&str]) -> Context {
    let mut admission = DeltaAdmission::new(defs.iter().map(|def| {
        (
            def.name.to_string(),
            DeltaCandidate {
                ty: &def.ty,
                body: &def.body,
                total: def.total,
                decreasing: None,
            },
        )
    }));
    let mut base = Context::new();
    for def in defs.iter().filter(|def| roots.contains(&def.name)) {
        let marker = if def.total {
            GlobalAdmission::Admitted
        } else {
            GlobalAdmission::Opaque
        };
        base = base.with_global(def.name, def.ty.clone(), marker);
    }
    let roots: BTreeSet<String> = roots.iter().map(|root| (*root).to_string()).collect();
    admission.bind(&base, &roots)
}

fn two() -> Def {
    def("two", Type::Nat, Term::NatLit(2))
}

/// `length`, its self-call a free `Global` exactly as the elaborator hands it.
fn length() -> Def {
    def("length", list_to_nat(), length_body(Term::global("length")))
}

fn length_of(list: Term) -> Term {
    Term::app(Term::global("length"), list)
}

/// `check_proof(ctx, refl left, Eq Nat left right)`.
fn proves(ctx: &Context, left: Term, right: Term) -> TypeResult<()> {
    check_proof(ctx, &refl_nat(left.clone()), &eq_nat(left, right))
}

// ==========================================================================
// D6 — the must-fail twins
// ==========================================================================

#[test]
fn a_partial_body_is_never_unfolded() {
    let ctx = bound(&[two().partial()], &["two"]);
    assert!(proves(&ctx, Term::global("two"), Term::NatLit(2)).is_err());
}

// Taint: `wrapper` reaches `#[partial]` `spin`, so the report says `Partial`
// for both, and neither unfolds — even though `wrapper`'s own body is total-
// looking and `spin` is not even in the theorem.
#[test]
fn a_tainted_body_is_never_unfolded() {
    let spin = def("spin", Type::Nat, Term::NatLit(2)).partial();
    let wrapper = def("wrapper", Type::Nat, Term::global("spin")).partial();
    let ctx = bound(&[spin, wrapper], &["wrapper"]);
    assert!(proves(&ctx, Term::global("wrapper"), Term::NatLit(2)).is_err());
}

/// `let _ = <extern call> in 2`: unfolding it would prove `noisy == 2`.
fn extern_then_two(symbol: &str) -> Term {
    Term::Let(
        "_".into(),
        Type::Unit,
        Box::new(Term::extern_call(symbol, vec![])),
        Box::new(Term::NatLit(2)),
    )
}

#[test]
fn an_effectful_body_is_never_unfolded() {
    let noisy = def("noisy", Type::Nat, extern_then_two("print_line"));
    let ctx = bound(&[noisy], &["noisy"]);
    assert!(proves(&ctx, Term::global("noisy"), Term::NatLit(2)).is_err());
}

// D3.2's deliberate divergence from the elaborator's δ: a wrapper over an
// extern 3.10.26c exempts as pure is still neutral to the kernel.
#[test]
fn a_pure_extern_wrapper_is_not_unfolded() {
    let pure = def("pure", Type::Nat, extern_then_two("tg_nat_to_string"));
    let ctx = bound(&[pure], &["pure"]);
    assert!(proves(&ctx, Term::global("pure"), Term::NatLit(2)).is_err());
}

#[test]
fn an_ill_typed_body_is_not_admitted() {
    let liar = def("two", Type::Nat, Term::True);
    let ctx = bound(&[liar], &["two"]);
    assert!(proves(&ctx, Term::global("two"), Term::NatLit(2)).is_err());
    assert!(ctx.unfoldings().is_empty());
}

// `grow(l) = grow(Cons(0, l))`: well typed, never descends.
#[test]
fn a_non_terminating_fix_is_not_admitted() {
    let grow = def(
        "grow",
        list_to_nat(),
        list_fn(
            "l",
            Term::app(
                Term::global("grow"),
                crate::terms::termination::tests::fixtures::cons(Term::Zero, var("l")),
            ),
        ),
    );
    let ctx = bound(&[grow], &["grow"]);
    assert!(ctx.unfoldings().is_empty());
    assert!(proves(
        &ctx,
        Term::app(Term::global("grow"), list_of(&[])),
        Term::Zero
    )
    .is_err());
}

// `f(n) = f(n)`, its self-call a free `Global`, marked `Total` by a
// hand-built report: the kernel abstracts the call and `fix_admission`
// refuses it — the report's word is not taken for a self-recursive body.
#[test]
fn an_elaborated_self_call_is_rechecked() {
    let looping = def(
        "f",
        Type::arrow(list_type(), Type::Nat),
        list_fn("n", Term::app(Term::global("f"), var("n"))),
    );
    let ctx = bound(&[looping], &["f"]);
    assert!(ctx.unfoldings().is_empty());
    assert!(proves(&ctx, Term::app(Term::global("f"), list_of(&[])), Term::Zero).is_err());
}

// A self-mentioning body that also calls a sibling is still a singleton
// component: the kernel classifies it, so it gets `fix` and its re-check.
#[test]
fn a_self_recursive_body_shaped_as_mutual_is_rechecked() {
    let helper = def("helper", list_to_nat(), list_fn("l", Term::Zero));
    let looping = def(
        "f",
        list_to_nat(),
        list_fn(
            "l",
            plus(
                Term::app(Term::global("f"), var("l")),
                Term::app(Term::global("helper"), var("l")),
            ),
        ),
    );
    let ctx = bound(&[helper, looping], &["f"]);
    let unfoldings = ctx.unfoldings();
    assert!(unfoldings.body("f").is_none());
    assert!(unfoldings.body("helper").is_some());
}

// `length`'s body binds a local named `length`: the abstraction's binder is
// fresh, so the local still means the local and the call still recurses.
#[test]
fn a_shadowing_local_is_not_captured() {
    let body = list_fn(
        "l",
        Term::Let(
            "length".into(),
            Type::Nat,
            Box::new(Term::NatLit(5)),
            Box::new(match_list(
                var("l"),
                var("length"),
                "t",
                plus(
                    Term::succ(Term::Zero),
                    Term::app(Term::global("length"), var("t")),
                ),
            )),
        ),
    );
    let ctx = bound(&[def("length", list_to_nat(), body)], &["length"]);
    assert_eq!(
        proves(&ctx, length_of(list_of(&[])), Term::NatLit(5)),
        Ok(())
    );
    assert_eq!(
        proves(&ctx, length_of(list_of(&[9])), Term::NatLit(6)),
        Ok(())
    );
    assert!(proves(&ctx, length_of(list_of(&[9])), Term::NatLit(1)).is_err());
}

/// `ping(n) = pong(n)`, `pong(n) = ping(n)`: a mutual pair, self-bound on the
/// report's `Total` marker, that never reaches a value.
fn ping_pong() -> [Def; 2] {
    let calls = |callee: &str| list_fn("n", Term::app(Term::global(callee), var("n")));
    [
        def("ping", list_to_nat(), calls("pong")),
        def("pong", list_to_nat(), calls("ping")),
    ]
}

#[test]
fn a_mutual_member_on_a_neutral_is_refused_within_budget() {
    let ctx = bound(&ping_pong(), &["ping"]).with_term("xs", list_type());
    assert!(ctx.unfoldings().body("ping").is_some());
    let verdict = proves(&ctx, Term::app(Term::global("ping"), var("xs")), Term::Zero);
    assert!(matches!(verdict, Err(TypeError::TypeMismatch { .. })));
}

#[test]
fn a_false_equality_is_refused_after_unfolding() {
    let ctx = bound(&[two()], &["two"]);
    assert!(proves(&ctx, Term::global("two"), Term::NatLit(3)).is_err());
    let ctx = bound(&[length()], &["length"]);
    assert!(proves(&ctx, length_of(list_of(&[1])), Term::NatLit(2)).is_err());
}

#[test]
fn unfolding_that_exhausts_its_budget_is_refused() {
    let ctx = bound(&ping_pong(), &["ping"]);
    let verdict = proves(
        &ctx,
        Term::app(Term::global("ping"), list_of(&[1])),
        Term::Zero,
    );
    assert!(matches!(verdict, Err(TypeError::TypeMismatch { .. })));
}

// ==========================================================================
// AC 1 — what the kernel does unfold
// ==========================================================================

#[test]
fn an_admitted_body_unfolds() {
    let ctx = bound(&[two()], &["two"]);
    assert_eq!(proves(&ctx, Term::global("two"), Term::NatLit(2)), Ok(()));
}

#[test]
fn a_self_recursive_body_unfolds_as_fix() {
    let ctx = bound(&[length()], &["length"]);
    let unfoldings = ctx.unfoldings();
    assert!(matches!(unfoldings.body("length"), Some(Term::Fix(..))));
    assert_eq!(unfoldings.decreasing("length"), Some(0));
    assert_eq!(
        proves(&ctx, length_of(list_of(&[1, 2])), Term::NatLit(2)),
        Ok(())
    );
}

// A body reached only through another body's closure is admitted too, and
// dependencies first: `four`'s check unfolds `two`.
#[test]
fn a_dependency_is_admitted_before_its_dependent() {
    let four = def(
        "four",
        Type::Nat,
        plus(Term::global("two"), Term::global("two")),
    );
    let ctx = bound(&[two(), four], &["four"]);
    assert_eq!(proves(&ctx, Term::global("four"), Term::NatLit(4)), Ok(()));
}

#[test]
fn an_opaque_rebinding_shadows_an_admitted_body() {
    let ctx = bound(&[two()], &["two"]);
    let inner = ctx.with_global("two", Type::Nat, GlobalAdmission::Opaque);
    assert!(inner.unfoldings().body("two").is_none());
    assert!(proves(&inner, Term::global("two"), Term::NatLit(2)).is_err());
}

#[test]
fn executable_mode_still_does_not_unfold() {
    let ctx = bound(&[two()], &["two"]);
    let left = Term::global("two");
    assert!(check(
        &ctx,
        &refl_nat(left.clone()),
        &eq_nat(left, Term::NatLit(2))
    )
    .is_err());
}

// ==========================================================================
// AC 3 — an empty context converts exactly as before
// ==========================================================================

#[test]
fn an_empty_context_converts_as_before() {
    assert!(Context::new().unfoldings().is_empty());
    assert!(over_n().unfoldings().is_empty());
    // A plain `Admitted` marker carries no body: `two` stays neutral.
    let marker_only = Context::new().with_global("two", Type::Nat, GlobalAdmission::Admitted);
    assert!(marker_only.unfoldings().is_empty());
    assert!(proves(&marker_only, Term::global("two"), Term::NatLit(2)).is_err());
    assert_eq!(
        check_proof(
            &Context::new(),
            &refl_nat(plus(Term::NatLit(1), Term::NatLit(1))),
            &eq_nat(plus(Term::NatLit(1), Term::NatLit(1)), Term::NatLit(2)),
        ),
        Ok(())
    );
}
