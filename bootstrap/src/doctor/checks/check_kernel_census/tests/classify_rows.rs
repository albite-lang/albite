//! D6's table and its first-match order (ADR 30.9.26f AC 1).

use super::*;

/// 30.9.26f AC1: one injected refusal per row of D6's table.
#[test]
fn classify_names_each_cause_from_injected_refusals() {
    let none = Probes::default();
    let yes = |set: fn(&mut Probes)| {
        let mut probes = Probes::default();
        set(&mut probes);
        probes
    };
    let cases = [
        (TypeError::SorryInProof, none, Cause::Hole),
        (TypeError::ReturnInProof, none, Cause::ExecutableConstruct),
        (
            TypeError::FixNotStructural {
                name: "f".into(),
                reason: "r".into(),
            },
            none,
            Cause::ExecutableConstruct,
        ),
        (
            TypeError::PoisonInProof { ty: Type::Error },
            none,
            Cause::Poison,
        ),
        (
            TypeError::GlobalNotAdmitted { name: "g".into() },
            none,
            Cause::GlobalNotAdmitted,
        ),
        (
            TypeError::UnboundVariable("g".into()),
            none,
            Cause::MissingGlobalContext,
        ),
        (
            TypeError::UnboundTypeVar("@List".into()),
            yes(|p| p.marker = Some(true)),
            Cause::MarkerIllFormed,
        ),
        (
            TypeError::MalformedType(Type::TyVar("List".into())),
            yes(|p| p.names_concrete_type = Some(true)),
            Cause::NamedTypeReference,
        ),
        (
            mismatch(Type::Nat, Type::Bool),
            probes(Some(true), None),
            Cause::StrictEquality,
        ),
        // Row 9's two keys went with the `Prop` type (ADR 4.10.26i D2).
        (
            mismatch(Type::Nat, Type::Bool),
            probes(Some(false), Some(true)),
            Cause::NeedsConversion,
        ),
        (
            mismatch(Type::Nat, Type::Bool),
            probes(Some(false), Some(false)),
            Cause::UnexplainedMismatch,
        ),
        (TypeError::EmptyMatch, none, Cause::Unclassified),
    ];
    for (refusal, probes, cause) in cases {
        assert_eq!(classify(&refusal, &probes), cause, "{refusal:?}");
    }
}

/// 6.10.26h D6: every predicate-binder refusal has its own row, ahead of
/// "unclassified", whatever the probes say.
#[test]
fn every_predicate_binder_refusal_is_its_own_row() {
    let var = || "P".to_string();
    let refusals = [
        TypeError::PredicateBinderNotPrenex { ty: Type::Nat },
        TypeError::StrayPredicate { ty: Type::Nat },
        TypeError::PredicateKindEmpty { var: var() },
        TypeError::PredicateDomainNotData {
            var: var(),
            domain: Type::Nat,
        },
        TypeError::PredicateDomainOpen {
            var: var(),
            domain: Type::Nat,
        },
        TypeError::PredicateInstantiatesForall { got: Type::Nat },
        TypeError::NotAPredicate { got: Type::Nat },
        TypeError::PredicateKindMismatch {
            expected: vec![],
            got: Type::Nat,
        },
        TypeError::UnboundPredicateVar(var()),
        TypeError::PredicateVarIsTypeVar(var()),
        TypeError::PredicateArity {
            var: var(),
            expected: 1,
            got: 2,
        },
        TypeError::PredicateIntroShape { ty: Type::Nat },
    ];
    let lenient = probes(Some(true), Some(true));
    for refusal in refusals {
        assert_eq!(
            classify(&refusal, &lenient),
            Cause::PredicateBinder,
            "{refusal:?}"
        );
        assert!(!refusal.to_string().is_empty());
    }
}

/// 30.9.26f AC1: each of the five mismatch variants reaches rows 8–11.
#[test]
fn every_mismatch_variant_is_in_the_mismatch_set() {
    let (expected, got) = (Type::Nat, Type::Bool);
    let variants = [
        mismatch(expected.clone(), got.clone()),
        TypeError::ArgumentTypeMismatch {
            expected: expected.clone(),
            got: got.clone(),
        },
        TypeError::NatIndMotiveMismatch {
            expected: expected.clone(),
            got: got.clone(),
        },
        TypeError::NatRecSuccTypeMismatch {
            expected: expected.clone(),
            got: got.clone(),
        },
        TypeError::BranchTypeMismatch {
            then_type: expected,
            else_type: got,
        },
    ];
    for refusal in variants {
        let converts = probes(Some(false), Some(true));
        assert_eq!(
            classify(&refusal, &converts),
            Cause::NeedsConversion,
            "{refusal:?}"
        );
        assert_eq!(
            classify(&refusal, &Probes::default()),
            Cause::UnexplainedMismatch
        );
    }
    // 4.10.26i D2: a data-against-`Eq` branch pair, row 9's old shape with
    // `Unit` where `Prop` stood, is no longer singled out.
    let branch = TypeError::BranchTypeMismatch {
        then_type: Type::Unit,
        else_type: eq_nat(Term::Zero, Term::Zero),
    };
    assert_eq!(
        classify(&branch, &Probes::default()),
        Cause::UnexplainedMismatch
    );
}

/// 30.9.26f AC1: where rows overlap, the earlier row wins.
#[test]
fn classify_order_is_first_match() {
    let both = probes(Some(true), Some(true));
    assert_eq!(
        classify(&mismatch(Type::Nat, Type::Bool), &both),
        Cause::StrictEquality
    );
    let unit_side = mismatch(eq_nat(Term::Zero, Term::Zero), Type::Unit);
    assert_eq!(
        classify(&unit_side, &probes(Some(false), Some(true))),
        Cause::NeedsConversion
    );
    let marker_and_named = Probes {
        marker: Some(true),
        names_concrete_type: Some(true),
        ..Probes::default()
    };
    let unbound = TypeError::UnboundTypeVar("@List".into());
    assert_eq!(
        classify(&unbound, &marker_and_named),
        Cause::MarkerIllFormed
    );
}

/// 30.9.26f AC1: row 11 — a mismatch no probe explains.
#[test]
fn a_mismatch_no_probe_explains_is_unexplained() {
    let refusal = mismatch(Type::Nat, Type::Bool);
    assert_eq!(
        classify(&refusal, &probes(Some(false), Some(false))),
        Cause::UnexplainedMismatch
    );
}

/// 30.9.26f AC1: row 12 — outside every listed variant.
#[test]
fn an_unlisted_error_is_unclassified() {
    let not_a_function = TypeError::NotAFunction { got: Type::Nat };
    assert_eq!(
        classify(&not_a_function, &probes(Some(true), Some(true))),
        Cause::Unclassified
    );
    let not_equality = TypeError::NotEquality { got: Type::Nat };
    assert_eq!(
        classify(&not_equality, &Probes::default()),
        Cause::Unclassified
    );
}

/// 30.9.26f AC1: with every probe `None`, rows 6–10 cannot fire on a
/// non-`Prop` error.
#[test]
fn classify_reads_only_injected_probes() {
    let none = Probes::default();
    let unbound = TypeError::UnboundTypeVar("@List".into());
    assert_eq!(classify(&unbound, &none), Cause::Unclassified);
    let malformed = TypeError::MalformedType(Type::TyVar("List".into()));
    assert_eq!(classify(&malformed, &none), Cause::Unclassified);
    assert_eq!(
        classify(&mismatch(Type::Nat, Type::Nat), &none),
        Cause::UnexplainedMismatch
    );
}

/// 30.9.26f AC1: the labels are the ones the record and D8's line print, and
/// only rows 1–3 mask the rest of a proof (D5).
#[test]
fn cause_labels_and_masking_rows() {
    let labels: Vec<&str> = Cause::ALL.iter().map(|cause| cause.label()).collect();
    assert_eq!(
        labels,
        [
            "admitted",
            "axiom",
            "hole",
            "executable-construct",
            "poison",
            "global-not-admitted",
            "missing-global-context",
            "marker-ill-formed",
            "named-type-reference",
            "strict-equality",
            "dependent-subst",
            "needs-conversion",
            "unexplained-mismatch",
            "predicate-binder",
            "unclassified",
        ]
    );
    let masking: Vec<Cause> = Cause::ALL
        .into_iter()
        .filter(|cause| cause.masks_the_rest())
        .collect();
    assert_eq!(
        masking,
        [Cause::Hole, Cause::ExecutableConstruct, Cause::Poison]
    );
}

/// 4.10.26i D6 (b): a statement that sorts `Type₀` is reported, a proposition
/// and a poison are not.
#[test]
fn a_statement_that_sorts_type0_states_data() {
    assert!(states_data(&Type::Nat));
    assert!(states_data(&Type::arrow(Type::Nat, Type::Nat)));
    assert!(!states_data(&eq_nat(Term::Zero, Term::Zero)));
    assert!(!states_data(&Type::arrow(Type::Nat, Type::Void)));
    assert!(!states_data(&Type::Error));
}

/// 4.10.26i D6 (b): the corpus line names the data statements, and a clean
/// line is unchanged.
#[test]
fn the_corpus_line_names_data_statements() {
    use crate::doctor::checks::check_kernel_census::corpus::data_statements_suffix;
    assert_eq!(data_statements_suffix(&[]), "");
    let names = ["thm_colon".to_string(), "zero_is_nat".to_string()];
    assert_eq!(
        data_statements_suffix(&names),
        " — sort Type₀: thm_colon, zero_is_nat"
    );
}

/// 4.10.26i D6 (b): the report counts data statements and marks each row.
#[test]
fn render_census_marks_data_statements() {
    let admitted = |name: &str| CensusRow {
        name: name.into(),
        primary: Cause::Admitted,
        secondary: None,
        refusal: None,
    };
    let rows = [admitted("data_thm"), admitted("prop_thm")];
    let tally = CensusTally {
        proof_relevant: 2,
        admitted: 2,
        data_statements: vec!["data_thm".into()],
        ..CensusTally::default()
    };
    let report = render_census("f.tg", &tally, &rows, CensusRows::All);
    assert!(
        report.contains("  sort Type₀:  1   (statements that state data"),
        "{report}"
    );
    assert!(
        report.contains("  data_thm: admitted / — [sort: Type₀]\n"),
        "{report}"
    );
    assert!(report.contains("  prop_thm: admitted / —\n"), "{report}");
    let clean = CensusTally {
        data_statements: Vec::new(),
        ..tally
    };
    assert!(!render_census("f.tg", &clean, &rows, CensusRows::All).contains("sort"));
}

/// The census over `source` as a one-file project, in the command's own
/// window, without `--include-total`.
fn census_of(source: &str) -> (CensusTally, Vec<CensusRow>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let main = dir.path().join("main.tg");
    std::fs::write(&main, source).expect("write");
    let reporting = crate::elaborate::termination::ReportingOnly::begin();
    let project = crate::driver::elaborate_project(&main, false, 20, None).expect("it elaborates");
    drop(reporting);
    run_census(&project, false)
}

fn row<'r>(rows: &'r [CensusRow], name: &str) -> &'r CensusRow {
    rows.iter().find(|row| row.name == name).expect(name)
}

/// 13.10.26t AC 4: a proof-carrying `fn` is proof-relevant without
/// `--include-total`, so the census counts what the gate enforces: §1.1's
/// `bad` is refused and the theorem citing it is admitted.
#[test]
fn a_proof_fn_is_in_the_population_without_include_total() {
    let (tally, rows) = census_of(
        "fn big() -> Nat { 18446744073709551615 }\n\
         fn bad() -> Eq<Nat, big() + 1, big()> { refl }\n\
         theorem cites(): Eq<Nat, big() + 1, big()> { bad() }\n",
    );
    assert_eq!((tally.proof_relevant, tally.total_only), (2, 0));
    assert_eq!(row(&rows, "bad").primary, Cause::UnexplainedMismatch);
    assert_eq!(row(&rows, "cites").primary, Cause::Admitted);
    assert!(
        rows.iter().all(|row| row.name != "big"),
        "data is not a proof"
    );
}

/// 13.10.26t AC 4: §1.1's `hole`/`anything` file is unchanged in what it
/// says of `anything` (D2): `hole` is a hole, and `anything` still reads
/// admitted, so a later change there is visible.
#[test]
fn a_theorem_resting_on_a_sorry_fn_still_reads_admitted() {
    let (tally, rows) = census_of(
        "fn hole() -> Void { sorry }\n\
         theorem anything(): Eq<Nat, 0, 1> { absurd(hole()) }\n",
    );
    assert_eq!(tally.proof_relevant, 2);
    assert_eq!(row(&rows, "hole").primary, Cause::Hole);
    assert_eq!(row(&rows, "anything").primary, Cause::Admitted);
}

/// 13.10.26t AC 4: a `#[partial]` proof `fn` stays out of the population.
#[test]
fn a_partial_proof_fn_is_not_in_the_population() {
    let (tally, rows) = census_of(
        "#[partial]\nfn spin(n: Nat) -> Eq<Nat, n, n> { spin(n) }\n\
         theorem t(): Eq<Nat, 0, 0> { refl }\n",
    );
    assert_eq!(tally.proof_relevant, 1);
    assert!(rows.iter().all(|row| row.name != "spin"));
}
