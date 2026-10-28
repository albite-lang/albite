//! Substitution into a `natind` motive and into a type (ADR 29.9.26d AC 7).

use std::collections::HashSet;

use super::capture::fresh_name;
use crate::terms::Term;
use crate::types::Type;

fn eq_nat(left: Term, right: Term) -> Type {
    Type::eq(Type::Nat, left, right)
}

/// `natrec [Nat] zero (λi:Nat. λacc:Nat. body) (succ zero)` — a term that
/// binds `i` and `acc` around `body`.
fn binding_i_around(body: Term) -> Term {
    Term::natrec(
        Type::Nat,
        Term::Zero,
        Term::lambda("i", Type::Nat, Term::lambda("acc", Type::Nat, body)),
        Term::succ(Term::Zero),
    )
}

/// 29.9.26d AC7: `substitute_term` replaces inside `Eq` arguments, through
/// the type structure around them.
#[test]
fn substitute_term_replaces_inside_eq_arguments() {
    let body = Type::product(eq_nat(Term::var("k"), Term::Zero), Type::Nat);
    assert_eq!(
        body.substitute_term("k", &Term::var("n")),
        Type::product(eq_nat(Term::var("n"), Term::Zero), Type::Nat)
    );
    assert_eq!(Type::Nat.substitute_term("k", &Term::Zero), Type::Nat);
}

/// 29.9.26d AC7: an inner `natind` whose binder is `k` shadows it in its motive,
/// while its cases are still substituted.
#[test]
fn substitute_term_stops_at_an_inner_natind_with_the_same_binder() {
    let inner_motive = Type::arrow(Type::Nat, eq_nat(Term::var("k"), Term::var("k")));
    let inner = |case: Term| Term::natind("k", inner_motive.clone(), case, Term::Zero, Term::Zero);
    let body = eq_nat(inner(Term::var("k")), Term::Zero);
    assert_eq!(
        body.substitute_term("k", &Term::var("n")),
        eq_nat(inner(Term::var("n")), Term::Zero)
    );
}

/// 29.9.26d AC7 / P0 (b): a binder inside the body that would capture a free
/// variable of the replacement is renamed. Without the rename, `k := i` under
/// `λi` would make the body's `k` the lambda's own `i`.
#[test]
fn substitute_term_renames_a_capturing_binder() {
    let body = eq_nat(binding_i_around(Term::var("k")), Term::var("k"));
    let instantiated = body.substitute_term("k", &Term::var("i"));
    let captured = eq_nat(binding_i_around(Term::var("i")), Term::var("i"));
    assert_ne!(instantiated, captured, "the substitution captured `i`");
    let Type::Eq(_, left, right) = &instantiated else {
        panic!("expected an Eq, got {instantiated:?}");
    };
    assert_eq!(**right, Term::var("i"));
    assert!(
        left.free_vars().contains("i"),
        "the replacement's `i` stays free"
    );
    let Term::NatRec(_, _, step, _) = &**left else {
        panic!("expected a natrec, got {left:?}");
    };
    let Term::Lambda(renamed, _, _) = &**step else {
        panic!("expected a lambda, got {step:?}");
    };
    assert_ne!(renamed, "i");
}

/// 29.9.26d AC7: a binder that does not capture keeps its spelling, so a
/// restated annotation still compares equal.
#[test]
fn substitute_term_leaves_a_non_capturing_binder_alone() {
    let body = eq_nat(binding_i_around(Term::var("k")), Term::var("k"));
    assert_eq!(
        body.substitute_term("k", &Term::var("n")),
        eq_nat(binding_i_around(Term::var("n")), Term::var("n"))
    );
    // A binder named like a replacement variable, but with no `k` below it.
    let untouched = eq_nat(binding_i_around(Term::Zero), Term::var("k"));
    assert_eq!(
        untouched.substitute_term("k", &Term::var("i")),
        eq_nat(binding_i_around(Term::Zero), Term::var("i"))
    );
}

/// 29.9.26d AC7: `Term::substitute` into a `natind` whose binder differs from
/// `var` now enters the motive's `Eq` arguments.
#[test]
fn term_substitute_enters_a_natind_motive() {
    let motive = |outer: Term| Type::arrow(Type::Nat, eq_nat(outer, Term::var("k")));
    let node = |outer: Term| Term::natind("k", motive(outer), Term::Zero, Term::Zero, Term::Zero);
    assert_eq!(
        node(Term::var("m")).substitute("m", &Term::Zero),
        node(Term::Zero)
    );
    // A binder equal to `var` shadows it: the motive is left alone.
    assert_eq!(
        node(Term::var("k")).substitute("k", &Term::Zero),
        node(Term::var("k"))
    );
}

/// 29.9.26d AC7: a `natind` binder the replacement mentions is renamed first.
#[test]
fn term_substitute_renames_a_natind_binder_the_replacement_mentions() {
    let node = Term::natind(
        "k",
        Type::arrow(Type::Nat, eq_nat(Term::var("m"), Term::var("k"))),
        Term::Zero,
        Term::Zero,
        Term::Zero,
    );
    let Term::NatInd(binder, motive, ..) = node.substitute("m", &Term::var("k")) else {
        panic!("expected a natind");
    };
    assert_ne!(binder, "k");
    assert_eq!(
        motive,
        Type::arrow(Type::Nat, eq_nat(Term::var("k"), Term::var(binder)))
    );
}

/// 29.9.26d AC7: the free variables of a `natind` include its motive's, minus
/// its binder.
#[test]
fn natind_free_vars_include_the_motive_minus_its_binder() {
    let node = Term::natind(
        "k",
        Type::arrow(Type::Nat, eq_nat(Term::var("m"), Term::var("k"))),
        Term::var("z"),
        Term::Zero,
        Term::Zero,
    );
    let expected: HashSet<String> = ["m", "z"].into_iter().map(String::from).collect();
    assert_eq!(node.free_vars(), expected);
    assert_eq!(
        eq_nat(Term::var("m"), Term::var("k")).free_term_vars(),
        ["m", "k"].into_iter().map(String::from).collect()
    );
}

/// 29.9.26d AC7: a fresh name is primed until it is free.
#[test]
fn fresh_name_primes_until_unused() {
    let taken: HashSet<String> = ["i′".to_owned()].into_iter().collect();
    assert_eq!(fresh_name("i", &HashSet::new()), "i′");
    assert_eq!(fresh_name("i", &taken), "i′′");
}

/// 29.9.26d AC7: `substitute_avoiding` with a closed replacement is plain
/// substitution.
#[test]
fn substitute_avoiding_with_a_closed_replacement_is_plain_substitution() {
    let term = Term::lambda(
        "i",
        Type::Nat,
        Term::nat_add(Term::var("i"), Term::var("k")),
    );
    assert_eq!(
        term.substitute_avoiding("k", &Term::Zero),
        term.substitute("k", &Term::Zero)
    );
}

/// 29.9.26d AC7: every binding form renames when it would capture.
#[test]
fn substitute_avoiding_renames_every_binding_form() {
    let replacement = Term::var("x");
    let forms = [
        Term::let_in("x", Type::Nat, Term::var("k"), Term::var("k")),
        Term::case(Term::var("k"), "x", Term::var("k"), "y", Term::var("k")),
        Term::adt_match(
            Term::var("k"),
            vec![(0, "x".to_owned(), Box::new(Term::var("k")))],
        ),
        Term::fix("x", Type::Nat, Term::var("k")),
    ];
    for form in forms {
        let substituted = form.substitute_avoiding("k", &replacement);
        assert!(
            substituted.free_vars().contains("x"),
            "the replacement's `x` was captured in {substituted:?}"
        );
        assert!(
            !substituted.free_vars().contains("k"),
            "`k` survived in {substituted:?}"
        );
    }
}

// ── Dependent function types (ADR 30.9.26d)

/// `Π binder:domain. Eq<Nat, left, right>`
fn pi_eq(binder: &str, domain: Type, left: Term, right: Term) -> Type {
    Type::pi(binder, domain, eq_nat(left, right))
}

/// 30.9.26d AC5: substitution through a `Pi` reaches its codomain.
#[test]
fn substitute_term_reaches_a_pi_codomain() {
    let ty = pi_eq("m", Type::Nat, Term::var("n"), Term::var("m"));
    assert_eq!(
        ty.substitute_term("n", &Term::Zero),
        pi_eq("m", Type::Nat, Term::Zero, Term::var("m"))
    );
}

/// 30.9.26d AC5: the domain is substituted and a shadowed codomain is not.
#[test]
fn substitute_term_substitutes_the_domain_and_stops_at_a_shadowing_binder() {
    let three = Term::succ(Term::succ(Term::succ(Term::Zero)));
    let ty = Type::pi(
        "n",
        eq_nat(Term::var("n"), Term::Zero),
        eq_nat(Term::Zero, Term::var("n")),
    );
    assert_eq!(
        ty.substitute_term("n", &three),
        Type::pi(
            "n",
            eq_nat(three.clone(), Term::Zero),
            eq_nat(Term::Zero, Term::var("n")),
        )
    );
}

/// 30.9.26d AC5: a binder that would capture the replacement is renamed.
#[test]
fn substitute_term_renames_a_capturing_pi_binder() {
    // Π m. Eq<Nat, n + m, n + m>  [n := m]  must not capture the free `m`.
    let sum = |a: &str, b: &str| Term::nat_add(Term::var(a), Term::var(b));
    let ty = Type::pi("m", Type::Nat, eq_nat(sum("n", "m"), sum("n", "m")));
    let fresh = fresh_name("m", &HashSet::from(["m".to_string(), "n".to_string()]));
    assert_eq!(
        ty.substitute_term("n", &Term::var("m")),
        Type::pi(
            fresh.as_str(),
            Type::Nat,
            eq_nat(sum("m", &fresh), sum("m", &fresh))
        )
    );
}

/// 30.9.26d AC5: `fv(Pi(y, A, B)) = fv(A) ∪ (fv(B) \ {y})`.
#[test]
fn free_term_vars_drops_the_pi_binder() {
    let ty = Type::pi(
        "y",
        eq_nat(Term::var("a"), Term::Zero),
        eq_nat(Term::var("y"), Term::var("b")),
    );
    assert_eq!(
        ty.free_term_vars(),
        HashSet::from(["a".to_string(), "b".to_string()])
    );
    let binder_in_domain = Type::pi("y", eq_nat(Term::var("y"), Term::Zero), Type::Nat);
    assert_eq!(
        binder_in_domain.free_term_vars(),
        HashSet::from(["y".to_string()])
    );
}

/// 30.9.26d AC3: `pi_or_arrow` builds a `Pi` only when the binder is free.
#[test]
fn pi_or_arrow_keeps_a_non_dependent_arrow() {
    let closed = eq_nat(Term::Zero, Term::Zero);
    assert_eq!(
        Type::pi_or_arrow("n", Type::Nat, closed.clone()),
        Type::arrow(Type::Nat, closed)
    );
    let open = eq_nat(Term::var("n"), Term::var("n"));
    assert_eq!(
        Type::pi_or_arrow("n", Type::Nat, open.clone()),
        Type::pi("n", Type::Nat, open)
    );
}

/// 30.9.26d AC5: erasure leaves no `Pi` and preserves `Eq`.
#[test]
fn erase_dependency_rewrites_every_pi_and_keeps_eq() {
    let body = eq_nat(Term::var("n"), Term::var("n"));
    let nested = Type::product(Type::pi("n", Type::Nat, body.clone()), Type::Nat);
    let erased = nested.erase_dependency();
    assert_eq!(
        erased,
        Type::product(Type::arrow(Type::Nat, body), Type::Nat)
    );
    assert!(nested.contains_dependent_former());
    assert!(!erased.contains_dependent_former());
    assert!(!Type::Nat.contains_dependent_former());
}

/// 30.9.26d AC5: `Term::erase_dependency` reaches a `Let`, a `Lambda`
/// annotation and a `TyApp` argument.
#[test]
fn term_erase_dependency_reaches_embedded_types() {
    let pi = Type::pi("n", Type::Nat, eq_nat(Term::var("n"), Term::var("n")));
    let term = Term::let_in(
        "f",
        pi.clone(),
        Term::global("lemma"),
        Term::lambda("g", pi.clone(), Term::ty_app(Term::var("g"), pi.clone())),
    );
    let erased = term.erase_dependency();
    let mut seen = Vec::new();
    collect_embedded(&erased, &mut seen);
    assert_eq!(seen.len(), 3);
    assert!(seen.iter().all(|ty| !ty.contains_dependent_former()));
    assert_eq!(seen[0], pi.erase_dependency());
}

fn collect_embedded(term: &Term, seen: &mut Vec<Type>) {
    term.for_each_embedded_type(|ty| seen.push(ty.clone()));
    term.for_each_subterm(|child| collect_embedded(child, seen));
}

/// 30.9.26d AC5: every structural walk treats a `Pi` like the arrow it
/// erases to, and names its binder when it prints.
#[test]
fn structural_walks_pass_through_a_pi() {
    use crate::terms::analysis::{former_of, Former};
    use crate::terms::termination::root_ineligibility;
    use crate::types::unfold_mu_type;

    let alpha = || Type::TyVar("a".into());
    let pi = Type::pi("n", alpha(), eq_nat(Term::var("n"), Term::var("n")));
    assert_eq!(pi.to_string(), "((n: a) → Eq Nat n n)");
    assert!(pi.display_detailed().starts_with("Pi(n, TyVar(a), Eq("));
    // Pi, its domain `a`, its codomain `Eq`, and `Eq`'s type argument `Nat`.
    assert_eq!(pi.node_count(), 4);
    assert_eq!(
        pi.substitute("a", &Type::Nat),
        Type::pi("n", Type::Nat, eq_nat(Term::var("n"), Term::var("n")))
    );
    assert_eq!(pi.free_type_vars(), HashSet::from(["a".to_string()]));
    assert!(!pi.is_well_formed(&HashSet::new()));
    assert!(pi.is_well_formed(&HashSet::from(["a".to_string()])));
    assert_eq!(former_of(&pi), Former::Arrow);
    assert_eq!(
        root_ineligibility(&pi, &[]),
        Some("a function type — there is no structure to take apart")
    );
    let recursive = Type::mu("a", Type::pi("n", alpha(), Type::Nat));
    assert_eq!(
        unfold_mu_type(&recursive),
        Type::pi("n", recursive.clone(), Type::Nat)
    );
}

/// 4.10.26h AC2: `substitute_term` under a `Sigma` renames the binder only on
/// capture, stops under a shadowing binder, and always reaches the domain.
#[test]
fn substitute_term_under_a_sigma_avoids_capture_and_respects_shadowing() {
    let body = eq_nat(Term::var("k"), Term::var("n"));
    let sigma = Type::sigma("k", Type::Nat, body.clone());
    assert_eq!(
        sigma.substitute_term("n", &Term::Zero),
        Type::sigma("k", Type::Nat, eq_nat(Term::var("k"), Term::Zero))
    );
    assert_eq!(sigma.substitute_term("k", &Term::Zero), sigma);
    let captured = sigma.substitute_term("n", &Term::var("k"));
    assert_eq!(
        captured,
        Type::sigma("k′", Type::Nat, eq_nat(Term::var("k′"), Term::var("k")))
    );
    assert_eq!(sigma.free_term_vars(), HashSet::from(["n".to_string()]));
}

/// 4.10.26h AC2: erasure maps a `Sigma` to its `Product`, and
/// `contains_dependent_former` sees both formers.
#[test]
fn erase_dependency_maps_a_sigma_to_its_product() {
    let body = eq_nat(Term::var("k"), Term::var("k"));
    let sigma = Type::sigma("k", Type::Nat, body.clone());
    assert_eq!(sigma.erase_dependency(), Type::product(Type::Nat, body));
    assert!(sigma.contains_dependent_former());
    assert!(Type::arrow(Type::Nat, sigma.clone()).contains_dependent_former());
    assert!(!sigma.erase_dependency().contains_dependent_former());
    assert_eq!(sigma.to_string(), "exists k: Nat. Eq Nat k k");
    assert!(sigma.display_detailed().starts_with("Sigma(k, Nat, Eq("));
}
