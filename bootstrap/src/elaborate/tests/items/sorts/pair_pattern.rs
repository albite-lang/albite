//! A `let` tuple pattern takes a dependent pair apart (ADR 6.10.26c AC 1, D7).
//!
//! Each lowering case runs the kernel in proof mode on the emitted term and
//! asserts the result is closed, embedded types included — the hazard the
//! rejected "substitute through the body" design had (D2). The refusals are
//! in `pair_pattern_refusals.rs`.

use std::collections::HashSet;

use super::def;
use crate::elaborate::tests::{elab_err, elab_ok};
use crate::elaborate::CoreDef;
use albite_core::typecheck::{check_proof, GlobalAdmission};
use albite_core::{Context, Term, Type};

pub(super) const LE: &str = "pub type Le(n: Nat, m: Nat): Prop = exists k: Nat. n + k == m\n\
                  pub theorem le_rebuilt(n: Nat, m: Nat, h: Le<n, m>) -> Le<n, m> {\n\
                      let (k, e) = h;\n\
                      (k, e)\n\
                  }\n";

/// The kernel context holding every definition elaborated before `name`.
fn kernel_context(defs: &[CoreDef]) -> Context {
    defs.iter().fold(Context::new(), |ctx, d| {
        ctx.with_global(d.name.clone(), d.ty.clone(), GlobalAdmission::Admitted)
    })
}

/// Elaborate `LE` + `source`, and require the kernel to admit `name` at its
/// stated type and its term to be closed, carried types included (AC 1).
pub(super) fn admitted(source: &str, name: &str) -> Term {
    let defs = elab_ok(&format!("{LE}{source}"));
    let target = def(&defs, name);
    assert_eq!(
        check_proof(&kernel_context(&defs), &target.term.term, &target.ty),
        Ok(()),
        "{name}: {}",
        target.term.term
    );
    let free = free_names(&target.term.term);
    assert!(
        free.is_empty(),
        "{name}: free {free:?} in {}",
        target.term.term
    );
    target.term.term.strip_spans()
}

/// Free term variables of `term`, counting every type it carries.
fn free_names(term: &Term) -> HashSet<String> {
    let mut free = HashSet::new();
    collect_free(term, &mut Vec::new(), &mut free);
    free
}

fn collect_free(term: &Term, bound: &mut Vec<String>, free: &mut HashSet<String>) {
    let of_type = |ty: &Type, bound: &Vec<String>, free: &mut HashSet<String>| {
        free.extend(
            ty.free_term_vars()
                .into_iter()
                .filter(|v| !bound.contains(v)),
        );
    };
    match term {
        Term::Var(v) if !bound.contains(v) => {
            free.insert(v.clone());
        }
        Term::Lambda(x, ty, body) | Term::Fix(x, ty, body) => {
            of_type(ty, bound, free);
            bound.push(x.clone());
            collect_free(body, bound, free);
            bound.pop();
        }
        Term::Let(x, ty, value, body) => {
            of_type(ty, bound, free);
            collect_free(value, bound, free);
            bound.push(x.clone());
            collect_free(body, bound, free);
            bound.pop();
        }
        Term::Annot(inner, ty) | Term::Refl(ty, inner) => {
            of_type(ty, bound, free);
            collect_free(inner, bound, free);
        }
        Term::Subst(base, motive, eq, proof) => {
            of_type(base, bound, free);
            of_type(motive, bound, free);
            collect_free(eq, bound, free);
            collect_free(proof, bound, free);
        }
        other => {
            let mut copy = other.clone();
            for child in copy.subterms_mut() {
                collect_free(child, bound, free);
            }
        }
    }
}

/// `term` under its parameter λs.
fn body_of(term: &Term) -> &Term {
    match term {
        Term::Lambda(_, _, body) => body_of(body),
        other => other,
    }
}

/// Whether any `let` in `term` binds a name starting `prefix`.
fn binds_let(term: &Term, prefix: &str) -> bool {
    if let Term::Let(x, ..) = term {
        if x.starts_with(prefix) {
            return true;
        }
    }
    let mut copy = term.clone();
    copy.subterms_mut()
        .into_iter()
        .any(|c| binds_let(c, prefix))
}

// 6.10.26c AC1 (D2): a witness named in a `let` annotation, an `Annot`'s
// type, a `refl`, and a λ annotation stays bound by the redex.
#[test]
fn the_witness_in_carried_types_stays_bound() {
    admitted(
        "pub theorem carried(n: Nat, m: Nat, h: Le<n, m>) -> Le<n, m> {\n\
             let (k, e) = h;\n\
             let e2: Eq<Nat, n + k, m> = e;\n\
             let w: exists j: Nat. Eq<Nat, k + j, k> = (0, refl);\n\
             let f = |q: Eq<Nat, n + k, m>| q;\n\
             (k, f(e2))\n\
         }",
        "carried",
    );
}

// 6.10.26c AC1 (D2): `cong` lowers to `subst` motives and a `refl` that name
// the witness; the kernel admits them.
#[test]
fn the_witness_in_a_subst_motive_stays_bound() {
    let term = admitted(
        "pub theorem succ_right(n: Nat, m: Nat, h: Le<n, m>) -> exists j: Nat. n + j == m + 1 {\n\
             let (k, e) = h;\n\
             (k + 1, cong(|x: Nat| x + 1, e))\n\
         }",
        "succ_right",
    );
    assert!(format!("{term}").contains("subst"), "{term}");
}

// 6.10.26c AC1 (D2): the emitted shape is `(λk. λe. body) (fst h) (snd h)`,
// and a variable scrutinee makes no `__tup`.
#[test]
fn a_variable_scrutinee_is_a_redex_over_its_projections() {
    let defs = elab_ok(LE);
    let core = body_of(&def(&defs, "le_rebuilt").term.term.strip_spans()).clone();
    let Term::App(inner, snd) = &core else {
        panic!("expected an application, got {core}");
    };
    assert_eq!(**snd, Term::snd(Term::var("h")));
    let Term::App(lambda, fst) = &**inner else {
        panic!("expected a nested application, got {inner}");
    };
    assert_eq!(**fst, Term::fst(Term::var("h")));
    assert!(matches!(&**lambda, Term::Lambda(k, Type::Nat, body)
        if k == "k" && matches!(&**body, Term::Lambda(e, Type::Eq(..), _) if e == "e")));
    assert!(!binds_let(&core, "__"), "{core}");
}

// 6.10.26c AC1 (D2): `(_, e)` binds `e` at `B[x := fst(h)]`, and only it.
#[test]
fn a_wildcard_witness_types_the_evidence_at_the_projection() {
    let term = admitted(
        "fn evidence_only(n: Nat, m: Nat, h: Le<n, m>) -> Nat { let (_, e) = h; 0 }",
        "evidence_only",
    );
    let Term::App(lambda, arg) = body_of(&term) else {
        panic!("expected an application, got {term}");
    };
    assert_eq!(**arg, Term::snd(Term::var("h")));
    let expected = Type::eq(
        Type::Nat,
        Term::nat_add(Term::var("n"), Term::fst(Term::var("h"))),
        Term::var("m"),
    );
    assert!(matches!(&**lambda, Term::Lambda(e, ty, _) if e == "e" && *ty == expected));
}

// 6.10.26c AC1 (D2): `(k, _)` binds the witness alone; `(_, _)` is the body.
#[test]
fn a_wildcard_evidence_binds_the_witness_alone() {
    let term = admitted(
        "fn witness_only(n: Nat, m: Nat, h: Le<n, m>) -> Nat { let (k, _) = h; k }",
        "witness_only",
    );
    assert_eq!(
        body_of(&term),
        &Term::app(
            Term::lambda("k", Type::Nat, Term::var("k")),
            Term::fst(Term::var("h"))
        )
    );
    let term = admitted(
        "fn neither(n: Nat, m: Nat, h: Le<n, m>) -> Nat { let (_, _) = h; 0 }",
        "neither",
    );
    assert!(
        matches!(body_of(&term), Term::Zero | Term::NatLit(0)),
        "{term}"
    );
}

// 6.10.26c AC1 (D2, D3): a non-variable scrutinee is bound once as `__tup`,
// and in check mode the redex is annotated with the expected type.
#[test]
fn a_non_variable_scrutinee_is_bound_and_annotated() {
    let term = admitted(
        "pub theorem via_call(n: Nat, m: Nat, h: Le<n, m>) -> Le<n, m> {\n\
             let (k, e) = le_rebuilt(n, m, h);\n\
             (k, e)\n\
         }",
        "via_call",
    );
    let Term::Let(tuple, _, _, body) = body_of(&term) else {
        panic!("expected `let __tup`, got {term}");
    };
    assert!(tuple.contains("__tup"), "{tuple}");
    assert!(matches!(&**body, Term::Annot(_, Type::Sigma(..))), "{body}");
}

// 6.10.26c AC1 (D3): check mode, a non-variable scrutinee, and a body whose
// inferred type names `k` but converts to the expected one: admitted.
#[test]
fn a_body_type_naming_the_witness_converts_under_the_annotation() {
    let term = admitted(
        "pub theorem times_zero(x: Nat) -> x * 0 == 0 { refl }\n\
         pub theorem zero_from_witness(n: Nat, m: Nat, h: Le<n, m>) -> Eq<Nat, 0, 0> {\n\
             let (k, _e) = le_rebuilt(n, m, h);\n\
             times_zero(k)\n\
         }",
        "zero_from_witness",
    );
    let Term::Let(_, _, _, body) = body_of(&term) else {
        panic!("expected `let __tup`, got {term}");
    };
    assert!(matches!(&**body, Term::Annot(_, Type::Eq(..))), "{body}");
}

// 6.10.26c AC1 (D2): a nested `Product` second part destructures the λ-bound
// `__e` with no fresh `__tup`.
#[test]
fn a_nested_product_destructures_the_bound_evidence() {
    let term = admitted(
        "pub theorem both(n: Nat, m: Nat, h: exists k: Nat. (Eq<Nat, n + k, m>, Eq<Nat, k, k>)) -> Le<n, m> {\n\
             let (k, (p, q)) = h;\n\
             (k, p)\n\
         }",
        "both",
    );
    assert!(!binds_let(&term, "____tup"), "{term}");
}

// 6.10.26c AC1 (D2): a nested `Sigma` second part is D2 again over `__e`.
#[test]
fn a_nested_sigma_destructures_the_bound_evidence() {
    let term = admitted(
        "pub theorem two(n: Nat, m: Nat, h: exists k: Nat. exists j: Nat. n + k + j == m)\n\
             -> exists k: Nat. exists j: Nat. n + k + j == m {\n\
             let (k, (j, e)) = h;\n\
             (k, (j, e))\n\
         }",
        "two",
    );
    assert!(!binds_let(&term, "__"), "{term}");
}

// 6.10.26c (D2): a part named like the scrutinee binds a fresh `__tup`, so
// `fst(h)`/`snd(h)` still name the pair.
#[test]
fn a_part_named_like_the_scrutinee_rebinds_it() {
    let term = admitted(
        "pub theorem shadow(n: Nat, m: Nat, h: Le<n, m>) -> Le<n, m> { let (k, h) = h; (k, h) }",
        "shadow",
    );
    assert!(binds_let(&term, "____tup"), "{term}");
}

// 6.10.26c AC1 (D1): the expression `let` form takes a pair apart too, in
// check mode and in infer mode.
#[test]
fn the_expression_let_form_destructures_a_pair() {
    let term = admitted(
        "pub theorem expr_form(n: Nat, m: Nat, h: Le<n, m>) -> Le<n, m> { (let (k, e) = h; (k, e)) }",
        "expr_form",
    );
    assert!(!binds_let(&term, "__"), "{term}");
    admitted(
        "fn expr_infer(n: Nat, m: Nat, h: Le<n, m>) -> Nat { let w = (let (k, _) = h; k); w }",
        "expr_infer",
    );
    let errors = elab_err(&format!(
        "{LE}fn expr_esc(n: Nat, m: Nat, h: Le<n, m>) -> Nat {{ let u = (let (k, e) = le_rebuilt(n, m, h); e); 0 }}"
    ));
    assert_eq!(errors[0].kind.code(), "E0011");
}

// The closedness check is not vacuous: it reports a free variable in term
// position and one inside a carried type, and not one bound by a λ.
#[test]
fn free_names_counts_carried_types_and_respects_binders() {
    let open = Term::lambda(
        "k",
        Type::Nat,
        Term::annot(
            Term::var("x"),
            Type::eq(Type::Nat, Term::var("y"), Term::var("k")),
        ),
    );
    let expected: HashSet<String> = ["x", "y"].into_iter().map(String::from).collect();
    assert_eq!(free_names(&open), expected);
}
