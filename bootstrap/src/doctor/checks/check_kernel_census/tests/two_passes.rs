//! D5's two passes over the pinned control (ADR 30.9.26f AC 2).

use super::*;

/// The `cong(|x: Nat| x + 1, ih)` step of `zero_add`, as the kernel's
/// `zero_add_step_is_known_refused_by_the_kernel` builds it.
fn zero_add_cong_step() -> Term {
    let plus_one = Term::lambda(
        "x",
        Type::Nat,
        Term::nat_add(Term::var("x"), Term::succ(Term::Zero)),
    );
    Term::subst(
        Type::Nat,
        Type::arrow(Type::Nat, Type::Unit),
        Term::var("h"),
        Term::refl(
            Type::Nat,
            Term::app(plus_one, Term::nat_add(Term::Zero, Term::var("j"))),
        ),
    )
}

fn zero_add_body(at: Term) -> Type {
    eq_nat(Term::nat_add(Term::Zero, at.clone()), at)
}

/// 30.9.26f AC2: a `sorry` base hides the step's `Subst` from proof mode. The
/// motive was `Nat → Prop` (row 9, `dependent-subst`) until the `Prop` type went
/// (ADR 4.10.26i D2); as `Nat → Unit` the step is an unexplained mismatch.
#[test]
fn the_zero_add_step_is_a_hole_masking_a_mismatch() {
    let step = Term::lambda(
        "j",
        Type::Nat,
        Term::lambda("h", zero_add_body(Term::var("j")), zero_add_cong_step()),
    );
    let term = Term::natind(
        "k",
        Type::arrow(Type::Nat, zero_add_body(Term::var("k"))),
        Term::annot(Term::Sorry, zero_add_body(Term::Zero)),
        step,
        Term::var("n"),
    );
    let ctx = Context::new().with_term("n", Type::Nat);
    let row = census_definition(
        "zero_add",
        &ctx,
        &term,
        &zero_add_body(Term::var("n")),
        &FixedProbes(Probes::default()),
    );
    assert_eq!(row.name, "zero_add");
    assert_eq!(row.primary, Cause::Hole);
    assert_eq!(row.secondary, Some(Cause::UnexplainedMismatch));
    assert_eq!(
        row.refusal.as_deref(),
        Some("`sorry` is not allowed in a proof")
    );
}

/// 30.9.26f AC2: the step case alone, under `j` and `h`.
#[test]
fn without_the_hole_the_zero_add_step_is_a_mismatch() {
    let ctx = Context::new()
        .with_term("j", Type::Nat)
        .with_term("h", zero_add_body(Term::var("j")));
    let goal = zero_add_body(Term::succ(Term::var("j")));
    let row = census_definition(
        "step",
        &ctx,
        &zero_add_cong_step(),
        &goal,
        &FixedProbes(Probes::default()),
    );
    assert_eq!(row.primary, Cause::UnexplainedMismatch);
    assert_eq!(row.secondary, None);
}

/// 30.9.26f AC2: a `FixNotStructural` primary is re-checked in executable mode.
#[test]
fn a_structural_failure_gets_a_secondary_cause() {
    let nat_to_nat = Type::arrow(Type::Nat, Type::Nat);
    let body = Term::lambda("x", Type::Nat, Term::app(Term::var("f"), Term::var("x")));
    let term = Term::fix("f", nat_to_nat.clone(), body);
    let row = census_definition(
        "loop",
        &Context::new(),
        &term,
        &nat_to_nat,
        &FixedProbes(Probes::default()),
    );
    assert_eq!(row.primary, Cause::ExecutableConstruct);
    assert_eq!(row.secondary, Some(Cause::Admitted));
}

/// 30.9.26f AC2: the secondary pass classifies with the engine's probes too.
#[test]
fn the_secondary_pass_reads_the_probe_engine() {
    let nat_to_nat = Type::arrow(Type::Nat, Type::Nat);
    let body = Term::lambda("x", Type::Nat, Term::app(Term::var("f"), Term::var("x")));
    let term = Term::fix("f", nat_to_nat, body);
    let converts = FixedProbes(probes(Some(false), Some(true)));
    let row = census_definition("loop", &Context::new(), &term, &Type::Nat, &converts);
    assert_eq!(row.primary, Cause::ExecutableConstruct);
    assert_eq!(row.secondary, Some(Cause::NeedsConversion));
}

/// 30.9.26f AC2: a closed `refl` types in proof mode, with nothing to add.
#[test]
fn a_refl_proof_of_a_closed_equality_is_admitted() {
    let term = Term::refl(Type::Nat, Term::Zero);
    let row = census_definition(
        "zero_is_zero",
        &Context::new(),
        &term,
        &eq_nat(Term::Zero, Term::Zero),
        &FixedProbes(Probes::default()),
    );
    assert_eq!(row.primary, Cause::Admitted);
    assert_eq!(row.secondary, None);
    assert_eq!(row.refusal, None);
}

/// 30.9.26f AC2: a refusal that masks nothing gets no second pass, whatever
/// the probes say.
#[test]
fn an_unmasking_refusal_gets_no_secondary() {
    let row = census_definition(
        "wrong",
        &Context::new(),
        &Term::Zero,
        &Type::Bool,
        &FixedProbes(probes(Some(false), Some(true))),
    );
    assert_eq!(row.primary, Cause::NeedsConversion);
    assert_eq!(row.secondary, None);
}
