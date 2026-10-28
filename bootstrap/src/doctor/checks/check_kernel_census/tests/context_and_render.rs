//! The fail-closed context, the exit codes and the report (ADR 30.9.26f
//! AC 3, AC 4).

use super::*;

fn admission_of(entries: &[(&str, AdmissionState)]) -> BTreeMap<String, AdmissionState> {
    entries
        .iter()
        .map(|(name, state)| (name.to_string(), *state))
        .collect()
}

fn mentioned(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| name.to_string()).collect()
}

// ── AC 3 — the context is fail-closed and minimal ──

/// 30.9.26f AC3
#[test]
fn bind_globals_marks_only_total_definitions_admitted() {
    let defs: HashMap<&str, &Type> =
        HashMap::from([("t", &Type::Nat), ("p", &Type::Nat), ("r", &Type::Nat)]);
    let admission = admission_of(&[
        ("t", AdmissionState::Total),
        ("p", AdmissionState::Partial),
        ("r", AdmissionState::Rejected),
    ]);
    let ctx = bind_globals(&mentioned(&["t", "p", "r"]), &defs, &admission);
    assert_eq!(
        ctx.lookup_global_admission("t"),
        Some(GlobalAdmission::Admitted)
    );
    assert_eq!(
        ctx.lookup_global_admission("p"),
        Some(GlobalAdmission::Opaque)
    );
    assert_eq!(
        ctx.lookup_global_admission("r"),
        Some(GlobalAdmission::Opaque)
    );
    assert_eq!(ctx.lookup_term("t"), Some(&Type::Nat));
}

/// 30.9.26f AC3
#[test]
fn a_mentioned_name_with_no_definition_stays_unbound() {
    let ctx = bind_globals(&mentioned(&["ghost"]), &HashMap::new(), &BTreeMap::new());
    let refusal = check_proof(&ctx, &Term::global("ghost"), &Type::Nat).unwrap_err();
    assert_eq!(refusal, TypeError::UnboundVariable("ghost".into()));
    assert_eq!(
        classify(&refusal, &Probes::default()),
        Cause::MissingGlobalContext
    );
}

/// 30.9.26f AC3
#[test]
fn an_unmentioned_definition_is_not_bound() {
    let defs: HashMap<&str, &Type> = HashMap::from([("used", &Type::Nat), ("unused", &Type::Bool)]);
    let admission = admission_of(&[
        ("used", AdmissionState::Total),
        ("unused", AdmissionState::Total),
    ]);
    let ctx = bind_globals(&mentioned(&["used"]), &defs, &admission);
    assert_eq!(ctx.len(), 1);
    assert_eq!(ctx.lookup_term("unused"), None);
}

/// 30.9.26f AC3: both node kinds, under a binder.
#[test]
fn mentioned_globals_sees_var_and_global() {
    let term = Term::lambda("x", Type::Nat, Term::app(Term::var("f"), Term::global("g")));
    assert_eq!(mentioned_globals(&term, &Type::Nat), mentioned(&["f", "g"]));
}

/// 1.10.26c AC6: a name only the statement's `Eq` mentions — or an `Eq`
/// inside an annotation the term embeds — is collected, and bound.
#[test]
fn mentioned_globals_reads_the_statement() {
    let statement = Type::pi(
        "n",
        Type::Nat,
        Type::eq(
            Type::Nat,
            Term::app(Term::global("double"), Term::var("n")),
            Term::var("n"),
        ),
    );
    let annotated = Term::lambda(
        "h",
        Type::eq(Type::Nat, Term::global("half"), Term::Zero),
        Term::Unit,
    );
    let collected = mentioned_globals(&annotated, &statement);
    assert_eq!(collected, mentioned(&["double", "half", "n"]));
    let defs: HashMap<&str, &Type> = HashMap::from([("double", &Type::Nat), ("half", &Type::Nat)]);
    let admission = admission_of(&[
        ("double", AdmissionState::Total),
        ("half", AdmissionState::Partial),
    ]);
    let ctx = bind_globals(&collected, &defs, &admission);
    assert_eq!(
        ctx.lookup_global_admission("double"),
        Some(GlobalAdmission::Admitted)
    );
    assert_eq!(
        ctx.lookup_global_admission("half"),
        Some(GlobalAdmission::Opaque)
    );
}

// ── AC 4 — exit codes and the rendered report ──

/// 30.9.26f AC4
#[test]
fn census_exit_is_zero_with_refusals() {
    let mut tally = CensusTally {
        proof_relevant: 2,
        ..CensusTally::default()
    };
    tally.record(Cause::Admitted, None);
    tally.record(Cause::Hole, Some(Cause::NeedsConversion));
    assert_eq!((tally.admitted, tally.refused), (1, 1));
    assert_eq!(tally.primary, BTreeMap::from([(Cause::Hole, 1)]));
    assert_eq!(
        tally.secondary,
        BTreeMap::from([(Cause::NeedsConversion, 1)])
    );
    assert_eq!(tally.exit_code(), 0);
    assert_eq!(tally.exit(), std::process::ExitCode::from(0));
}

/// 30.9.26f AC4
#[test]
fn census_exit_is_two_on_an_empty_population() {
    assert_eq!(CensusTally::default().exit_code(), 2);
    assert_eq!(
        CensusTally::default().exit(),
        std::process::ExitCode::from(2)
    );
    let total_only = CensusTally {
        total_only: 1,
        ..CensusTally::default()
    };
    assert_eq!((total_only.population(), total_only.exit_code()), (1, 0));
    let mut axioms = CensusTally {
        proof_relevant: 1,
        ..CensusTally::default()
    };
    axioms.record(Cause::Axiom, None);
    axioms.record(Cause::Axiom, None);
    assert_eq!(
        (axioms.axioms, axioms.refused, axioms.exit_code()),
        (2, 0, 0)
    );
    assert!(axioms.primary.is_empty());
}

fn refused_row() -> CensusRow {
    CensusRow {
        name: "zero_add".into(),
        primary: Cause::Hole,
        secondary: Some(Cause::DependentSubst),
        refusal: Some("sorry\nhere".into()),
    }
}

/// 30.9.26f AC4: the primary/secondary table and the `-v` row form.
#[test]
fn render_census_prints_the_table_and_verbose_rows() {
    let mut tally = CensusTally {
        proof_relevant: 2,
        total_only: 1,
        ..CensusTally::default()
    };
    tally.record(Cause::Hole, Some(Cause::DependentSubst));
    tally.record(Cause::Admitted, None);
    let admitted = CensusRow {
        name: "fine".into(),
        primary: Cause::Admitted,
        secondary: None,
        refusal: None,
    };
    let rows = [refused_row(), admitted];
    let quiet = render_census("f.tg", &tally, &rows, CensusRows::None);
    assert!(quiet.starts_with("kernel census: f.tg\n"));
    assert!(
        quiet.contains("population: 2 proof-relevant (+1 total with --include-total), 0 axioms")
    );
    assert!(quiet.contains("admitted:   1\n"));
    assert!(quiet.contains("refused:    1 "));
    assert!(quiet.contains("    hole                           1           0\n"));
    assert!(quiet.contains("    dependent-subst                0           1\n"));
    assert!(!quiet.contains("needs-conversion"));
    assert!(!quiet.contains("zero_add"));
    let verbose = render_census("f.tg", &tally, &rows, CensusRows::Refused);
    assert!(verbose.ends_with("  zero_add: hole / dependent-subst — sorry here\n"));
    assert!(!verbose.contains("fine:"));
}

/// 30.9.26f AC4: nothing refused prints no table; no secondary prints `—`.
#[test]
fn render_census_without_refusals_has_no_table() {
    let tally = CensusTally {
        proof_relevant: 1,
        admitted: 1,
        ..CensusTally::default()
    };
    assert!(!render_census("f.tg", &tally, &[], CensusRows::Refused).contains("cause"));
    let mut row = refused_row();
    row.secondary = None;
    let report = render_census(
        "f.tg",
        &CensusTally {
            refused: 1,
            ..tally
        },
        &[row],
        CensusRows::Refused,
    );
    assert!(report.contains("zero_add: hole / — —"));
}

/// One admitted, one axiom and one refused row, in name order, with the tally
/// they would record.
fn mixed_rows() -> (CensusTally, [CensusRow; 3]) {
    let admitted = CensusRow {
        name: "add_comm".into(),
        primary: Cause::Admitted,
        secondary: None,
        refusal: None,
    };
    let rows = [admitted, CensusRow::axiom("choice"), refused_row()];
    let mut tally = CensusTally {
        proof_relevant: 3,
        ..CensusTally::default()
    };
    for row in &rows {
        tally.record(row.primary, row.secondary);
    }
    (tally, rows)
}

/// The report's lines after its header and cause table: the per-row ones.
fn row_lines(report: &str) -> Vec<&str> {
    report
        .lines()
        .filter(|line| line.starts_with("  ") && !line.starts_with("    "))
        .filter(|line| {
            !["population:", "admitted:", "refused:"]
                .iter()
                .any(|h| line.trim_start().starts_with(h))
        })
        .collect()
}

/// 1.10.26m AC1 (D2, D3): `All` prints every row, each line complete and in
/// order — an admitted and an axiom row carry no refusal tail.
#[test]
fn render_census_with_all_rows_names_every_definition() {
    let (tally, rows) = mixed_rows();
    let report = render_census("f.tg", &tally, &rows, CensusRows::All);
    assert_eq!(
        row_lines(&report),
        [
            "  add_comm: admitted / —",
            "  choice: axiom / —",
            "  zero_add: hole / dependent-subst — sorry here",
        ]
    );
}

/// 1.10.26m AC2 (D1): the two flags' four combinations.
#[test]
fn census_rows_from_flags_is_d1s_table() {
    assert_eq!(CensusRows::from_flags(false, false), CensusRows::None);
    assert_eq!(CensusRows::from_flags(true, false), CensusRows::Refused);
    assert_eq!(CensusRows::from_flags(false, true), CensusRows::All);
    assert_eq!(CensusRows::from_flags(true, true), CensusRows::All);
}

/// 1.10.26m AC2 (D1): `None` and `Refused` are today's no-flag and `-v`
/// outputs, and every mode shares the counts and the cause table.
#[test]
fn render_census_without_all_rows_is_unchanged() {
    let (tally, rows) = mixed_rows();
    let render = |shown| render_census("f.tg", &tally, &rows, shown);
    let (quiet, verbose, all) = (
        render(CensusRows::None),
        render(CensusRows::Refused),
        render(CensusRows::All),
    );
    assert!(row_lines(&quiet).is_empty());
    assert_eq!(
        row_lines(&verbose),
        ["  zero_add: hole / dependent-subst — sorry here"]
    );
    assert_eq!(
        verbose,
        format!("{quiet}  zero_add: hole / dependent-subst — sorry here\n")
    );
    assert!(all.starts_with(&quiet));
    assert!(quiet.contains("admitted:   1\n") && quiet.contains("refused:    1 "));
    assert!(quiet.contains("    hole                           1           0\n"));
}

/// 1.10.26e AC5 (D6): a global named only inside a `Subst` motive's `Eq` —
/// as a `trans` motive can carry one — is collected; the motive's binder is
/// collected too, harmlessly, as a λ-bound name is.
#[test]
fn mentioned_globals_reads_a_subst_motive() {
    let motive = Type::pi(
        "#x",
        Type::Nat,
        Type::eq(Type::Nat, Term::global("lhs"), Term::var("#x")),
    );
    let term = Term::subst(Type::Nat, motive, Term::var("h"), Term::var("p"));
    assert_eq!(
        mentioned_globals(&term, &Type::Nat),
        mentioned(&["#x", "h", "lhs", "p"])
    );
}
