//! The shell's pure parts — the axiom discriminator and the probes — and one
//! elaborated golden end to end (ADR 30.9.26f P0.3, P0.4, AC 5).

use super::*;

fn core_def(term: Term, item_span: (u32, u32)) -> CoreDef {
    CoreDef {
        name: "a".into(),
        ty: Type::Nat,
        term: SpannedTerm::new(term, TermSpan::new(item_span.0, item_span.1)),
        span: Span {
            start: item_span.0,
            end: item_span.1,
        },
    }
}

/// 30.9.26f P0.3: an item-spanned `sorry` is an axiom; a token-spanned one is
/// an authored hole.
#[test]
fn an_axiom_is_a_sorry_spanned_by_its_whole_item() {
    let hole = |span: (u32, u32)| Term::spanned(Term::Sorry, TermSpan::new(span.0, span.1));
    let under_binders = Term::ty_abs("a", Term::lambda("h", Type::Nat, hole((0, 40))));
    assert!(is_axiom(&core_def(under_binders, (0, 40))));
    assert!(!is_axiom(&core_def(hole((30, 35)), (0, 40))));
    assert!(!is_axiom(&core_def(
        Term::spanned(Term::Zero, TermSpan::new(0, 40)),
        (0, 40)
    )));
    let wrapped = Term::spanned(hole((0, 40)), TermSpan::new(0, 40));
    assert!(is_axiom(&core_def(wrapped, (0, 40))));
    let generated = CoreDef {
        term: SpannedTerm::generated(hole((0, 40))),
        ..core_def(Term::Zero, (0, 40))
    };
    assert!(!is_axiom(&generated));
}

/// 30.9.26f D3: an axiom's row is counted, never checked.
#[test]
fn an_axiom_row_carries_no_refusal() {
    let row = CensusRow::axiom("choice");
    assert_eq!(row.name, "choice");
    assert_eq!(row.primary, Cause::Axiom);
    assert_eq!((row.secondary, row.refusal), (None, None));
}

fn golden(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/golden/check")
        .join(name)
}

/// 30.9.26f AC4 (D1): the command's three exits — a completed census is 0,
/// an empty population and a missing file are 2.
#[test]
fn the_command_exits_as_d1_says() {
    use std::process::ExitCode;
    let census = |name: &str, include_total| {
        cmd_check_kernel_census(&golden(name), include_total, CensusRows::Refused)
    };
    assert_eq!(
        census("conversion_unfolds_admitted_globals.tg", false),
        ExitCode::from(0)
    );
    // No theorem at all: empty without the flag, a census with it.
    let no_theorems = "conversion_reduces_under_a_variable.tg";
    assert_eq!(census(no_theorems, false), ExitCode::from(2));
    assert_eq!(census(no_theorems, true), ExitCode::from(0));
    assert_eq!(census("no_such_file.tg", false), ExitCode::from(2));
}

/// 30.9.26f AC4 (D1): the dispatcher returns the census's own exit code.
#[test]
fn the_dispatcher_passes_the_exit_through() {
    use crate::doctor::check_commands::check_proof::{dispatch_check_proof, CheckProofCommands};
    let command = CheckProofCommands::KernelCensus {
        file: Some(golden("no_such_file.tg")),
        corpus: Vec::new(),
        proof_bearing: false,
        include_total: false,
        all_rows: false,
    };
    assert_eq!(
        dispatch_check_proof(command, false),
        std::process::ExitCode::from(2)
    );
}

/// 30.9.26f D1: a population prints the report; an empty one prints only the
/// refusal, which names the flag exactly when it was not passed.
#[test]
fn census_report_refuses_an_empty_population() {
    let empty = CensusTally::default();
    let without_flag = census_report("f.tg", false, &empty, &[], CensusRows::None).unwrap_err();
    assert_eq!(
        without_flag,
        "error: no definition to check in f.tg — a census of nothing is not a clean one (try --include-total)"
    );
    let with_flag = census_report("f.tg", true, &empty, &[], CensusRows::None).unwrap_err();
    assert!(with_flag.ends_with("not a clean one"));
    let one = CensusTally {
        total_only: 1,
        admitted: 1,
        ..CensusTally::default()
    };
    let report = census_report("f.tg", true, &one, &[], CensusRows::None).expect("a report");
    assert!(report.starts_with("kernel census: f.tg\n"));
}

/// ADR 2.10.26d AC6: moving the classifier and the probes into
/// `elaborate::kernel` (D10) changes no byte of the census's report. The
/// fixtures are its `-v` output captured before the move, its counts since
/// grown by ADR 4.10.26g D5's three lemmas and ADR 6.10.26f D4's five.
/// Frozen on a corpus K-δ does not change: the δ golden's snapshot pinned
/// three refusals that ADR 2.10.26l removes by design, so that case went
/// with it.
#[test]
fn census_output_is_unchanged_by_the_move() {
    let cases = [(
        "src/compiler/nat_arith.tg",
        include_str!("before_the_move/nat_arith.txt"),
    )];
    for (file, before) in cases {
        let reporting = crate::elaborate::termination::ReportingOnly::begin();
        let project = elaborated(file);
        drop(reporting);
        let (tally, rows) = run_census(&project, false);
        let report = census_report(file, false, &tally, &rows, CensusRows::Refused);
        assert_eq!(report.as_deref(), Ok(before), "{file}");
    }
}

// ── AC 5 — one real corpus, end to end ──

/// 30.9.26f AC5: every theorem of the golden is in the population and
/// classified, and `via_two`'s context holds what it cites, as marked.
#[test]
fn census_over_conversion_unfolds_admitted_globals_checks_every_theorem() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/golden/check/conversion_unfolds_admitted_globals.tg");
    let reporting = crate::elaborate::termination::ReportingOnly::begin();
    let project =
        crate::driver::elaborate_project(&path, false, 20, None).expect("the golden elaborates");
    drop(reporting);

    let (tally, rows) = run_census(&project, false);
    let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
    // `length_one` is a proof `fn` (ADR 13.10.26t D4).
    let expected = "length_one length_two two_is_two two_plain via_two";
    assert_eq!(names.join(" "), expected);
    assert_eq!(
        (tally.proof_relevant, tally.total_only, tally.axioms),
        (5, 0, 0)
    );
    assert_eq!(tally.admitted + tally.refused, 5);
    assert!(rows.iter().all(|row| row.primary != Cause::Axiom));

    let admission =
        crate::elaborate::termination::TerminationInput::from_meta(&project.termination_meta)
            .check(&project.defs)
            .admission;
    let via_two = project
        .defs
        .iter()
        .find(|def| def.name == "via_two")
        .expect("via_two");
    let cited = mentioned_globals(&via_two.term.term, &via_two.ty);
    assert!(cited.contains("two_plain") && cited.contains("two_is_two"));
    let defs: HashMap<&str, &Type> = project
        .defs
        .iter()
        .map(|def| (def.name.as_str(), &def.ty))
        .collect();
    let ctx = bind_globals(&cited, &defs, &admission);
    for name in ["two_plain", "two_is_two"] {
        let marked = if admission.get(name) == Some(&AdmissionState::Total) {
            GlobalAdmission::Admitted
        } else {
            GlobalAdmission::Opaque
        };
        assert_eq!(ctx.lookup_global_admission(name), Some(marked), "{name}");
    }

    let with_total = run_census(&project, true).0;
    assert_eq!(with_total.proof_relevant, 5);
    assert!(
        with_total.total_only > 0,
        "`two` and `length` are admitted Total"
    );
}

/// Elaborate `path` (relative to the repo root) at the default enforcement.
/// Both corpora admit every definition, so no `ReportingOnly` window is needed
/// — though one would now be safe here: a window is scoped to the thread that
/// opened it, so it cannot reach another test's gate (ADR 2.10.26b D1).
fn elaborated(path: &str) -> crate::driver::output::ProjectOutput {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(path);
    crate::driver::elaborate_project(&path, false, 20, None).expect("the corpus elaborates")
}

/// Each row's primary cause, keyed by definition name, so a lookup can only
/// ever return the named row.
fn primaries(rows: &[CensusRow]) -> HashMap<&str, &Cause> {
    rows.iter()
        .map(|row| (row.name.as_str(), &row.primary))
        .collect()
}

fn primary_of<'r>(rows: &'r [CensusRow], name: &str) -> &'r Cause {
    primaries(rows)
        .get(name)
        .copied()
        .unwrap_or_else(|| panic!("{name} has a census row"))
}

/// 1.10.26e AC5: the Tier-1 rewrites — `add_comm`, and the three `natind`
/// proofs whose step is a `cong` — type in the kernel once `Subst` has a
/// dependent motive.
#[test]
fn census_types_the_tier_one_rewrites() {
    let project = elaborated("src/compiler/test_nat_lemmas.tg");
    let (_, rows) = run_census(&project, false);
    for name in ["add_comm", "add_zero_left", "add_succ_left", "add_assoc"] {
        let primary = primary_of(&rows, name);
        assert_ne!(primary, &Cause::DependentSubst, "{name}");
        assert_eq!(primary, &Cause::Admitted, "{name}");
    }
}

/// 1.10.26e AC5: `via_two`'s `trans` no longer stops at the `Subst`; its
/// citations' `Eq two 2` waited on K-δ, and since ADR 2.10.26l the kernel
/// unfolds `two`, so `via_two` is admitted outright.
#[test]
fn census_moves_via_two_off_dependent_subst() {
    let project = elaborated("tests/golden/check/conversion_unfolds_admitted_globals.tg");
    let (_, rows) = run_census(&project, false);
    assert_eq!(primary_of(&rows, "via_two"), &Cause::Admitted);
}

/// 1.10.26m AC3: `--all-rows` alone reaches every row end to end — each of
/// `nat_arith.tg`'s twenty-three theorems, admitted ones included, exactly once —
/// fifteen, since ADR 4.10.26g D5 `ring`'s three, and since ADR 6.10.26f D4
/// `omega`'s two and the three subtraction lemmas they are proved by.
#[test]
fn census_all_rows_lists_the_whole_library() {
    let library = "src/compiler/nat_arith.tg";
    let (tally, rows) = run_census(&elaborated(library), false);
    let shown = CensusRows::from_flags(false, true);
    let report = census_report(library, false, &tally, &rows, shown).expect("a report");
    let names = "add_zero_right add_succ_right add_zero_left add_succ_left add_assoc add_comm \
        mul_zero_right mul_succ_right mul_zero_left mul_one_right mul_succ_left mul_comm \
        mul_add add_mul mul_assoc mul_one_left add_left_comm mul_left_comm \
        nat_pred_sub_succ nat_sub_add_cancel nat_sub_zero_left add_right_cancel add_eq_zero_left";
    assert_eq!(names.split_whitespace().count(), 23);
    for name in names.split_whitespace() {
        let prefix = format!("  {name}: ");
        let lines = report.lines().filter(|line| line.starts_with(&prefix));
        assert_eq!(lines.count(), 1, "{name}");
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(library);
    let exit = cmd_check_kernel_census(&path, false, shown);
    assert_eq!(exit, std::process::ExitCode::from(0));
}

/// 6.10.26g AC1: every Tier 2 order lemma in `nat_order.tg` is listed once and
/// admitted. No total is asserted: the census also lists the imported
/// `nat_arith` rows (6.10.26g P0.4).
#[test]
fn census_admits_every_order_lemma() {
    let names = "le_refl le_add_right le_succ_self zero_le le_trans succ_le_succ \
        le_antisymm lt_succ_self le_of_lt lt_of_lt_of_le lt_of_le_of_lt lt_trans";
    assert_eq!(names.split_whitespace().count(), 12);
    assert_each_lemma_admitted_once("src/compiler/nat_order.tg", names);
}

/// 7.10.26l AC3: `le_total`, a disjunction proved by `left`/`right` and a
/// `cases` split on one, and its helper `le_step_cases` are admitted.
#[test]
fn census_admits_le_total_and_its_helper() {
    assert_each_lemma_admitted_once("src/compiler/nat_order.tg", "le_step_cases le_total");
}

/// 7.10.26m AC3: the refutations, each through `absurd(h)` on the kernel's
/// one trusted premise, `true ≠ false`, are admitted.
#[test]
fn census_admits_the_refutations() {
    let names = "true_ne_false succ_ne_zero succ_add_ne_self lt_irrefl not_succ_le_self";
    assert_eq!(names.split_whitespace().count(), 5);
    assert_each_lemma_admitted_once("src/compiler/nat_order.tg", names);
}

/// 7.10.26n AC3: strong induction and its five helpers, whose hypotheses are
/// written with the surface dependent function type, are admitted.
#[test]
fn census_admits_strong_induction() {
    let names = "not_lt_zero lt_succ_zero_eq lt_succ_succ_lt below_succ strong_ind_below \
        nat_strong_ind";
    assert_eq!(names.split_whitespace().count(), 6);
    assert_each_lemma_admitted_once("src/compiler/nat_order.tg", names);
}

/// 7.10.26j AC1: every Tier 3 divisibility lemma in `nat_divides.tg` is
/// listed once and admitted. No total, for the same reason as the order
/// library: the census lists the imported `nat_arith` rows too.
#[test]
fn census_admits_every_divisibility_lemma() {
    let names = "divides_refl divides_zero one_divides divides_trans divides_add \
        divides_mul_left divides_mul_right";
    assert_eq!(names.split_whitespace().count(), 7);
    assert_each_lemma_admitted_once("src/compiler/nat_divides.tg", names);
}

/// 7.10.26k AC1: `binom`'s Pascal rule and its base cases in `nat_binom.tg`
/// are each listed once and admitted. No total: the census lists the
/// imported `nat_arith` rows too.
/// 9.10.26j AC1 adds the diagonal and the zero above it; 9.10.26k AC1 the
/// absorption identity.
#[test]
fn census_admits_every_binomial_lemma() {
    let names = "binom_zero_succ binom_succ_zero binom_succ binom_zero_right binom_one_right \
        binom_zero_above binom_self binom_absorb";
    assert_eq!(names.split_whitespace().count(), 8);
    assert_each_lemma_admitted_once("src/compiler/nat_binom.tg", names);
}

/// 9.10.26j AC1: every D3 lemma of `nat_pow.tg` and `nat_sum.tg` is listed
/// once and admitted — the three `*_step` lemmas are the inductive sums'
/// steps, stated at the β-reduced summand. No total: the census lists the
/// imported `nat_arith` and `nat_order` rows too.
#[test]
fn census_admits_every_power_and_sum_lemma() {
    let powers = "pow_zero pow_succ pow_one nat_sub_zero_right nat_sub_succ_succ nat_sub_self";
    assert_eq!(powers.split_whitespace().count(), 6);
    assert_each_lemma_admitted_once("src/compiler/nat_pow.tg", powers);
    let sums = "sum_zero sum_succ sum_add_step sum_add sum_mul_left_step sum_mul_left \
        sum_shift_step sum_shift sum_congr";
    assert_eq!(sums.split_whitespace().count(), 9);
    assert_each_lemma_admitted_once("src/compiler/nat_sum.tg", sums);
}

/// 9.10.26i AC1: every D3 lemma of `nat_divmod.tg` — the equality test's,
/// decidable equality, the zero divisor, the four step lemmas, the bound and
/// `mod_def` — is listed once and admitted. `mod_lt_succ` is `mod_def_succ`'s
/// bound, proved by its own induction rather than as half of a pair. No
/// total: the census lists the imported `nat_arith` and `nat_order` rows too.
#[test]
fn census_admits_every_division_lemma() {
    let names = "nat_beq_succ_succ nat_beq_refl nat_beq_eq nat_beq_cases nat_eq_dec \
        nat_mod_zero_right nat_div_zero_right nat_mod_step_wrap nat_mod_step_keep \
        nat_div_step_wrap nat_div_step_keep lt_succ_of_beq_false mod_lt_succ mod_def_succ \
        mod_lt mod_def";
    assert_eq!(names.split_whitespace().count(), 16);
    assert_each_lemma_admitted_once("src/compiler/nat_divmod.tg", names);
}

/// 9.10.26k AC1: every D4 lemma of `nat_prime.tg` and the kept `mod_def`
/// route's helpers are listed once and admitted. No total: the census lists
/// the imported libraries' rows too. 9.10.26l AC1: `prime_divides_binom` and
/// its `subst` transport's helpers join them.
#[test]
fn census_admits_every_prime_lemma() {
    let lemmas = "prime_gt_one prime_divisor_cases divides_le not_divides_below divides_sub \
        euclid_lemma prime_divides_binom";
    assert_eq!(lemmas.split_whitespace().count(), 7);
    assert_each_lemma_admitted_once("src/compiler/nat_prime.tg", lemmas);
    let helpers = "prime_pos pos_of_ne_zero divides_of_zero_rem divides_rem_product \
        euclid_unit_or_self euclid_rem_cases euclid_below euclid_reduce \
        succ_of_lt_zero succ_of_lt_one binom_multiple";
    assert_each_lemma_admitted_once("src/compiler/nat_prime.tg", helpers);
}

/// 9.10.26m AC1: the binomial theorem, its step and the step's helpers in
/// `nat_binom_theorem.tg` are listed once and admitted. No total, as above.
#[test]
fn census_admits_the_binomial_theorem() {
    let names = "binomial_theorem binomial_step nat_sub_succ_lt nat_sub_add_self \
        binom_peel_split binom_left_half binom_right_half";
    assert_each_lemma_admitted_once("src/compiler/nat_binom_theorem.tg", names);
}

/// Each whitespace-separated name in `names` heads exactly one `admitted` row
/// of `library`'s `--all-rows` census report.
fn assert_each_lemma_admitted_once(library: &str, names: &str) {
    let (tally, rows) = run_census(&elaborated(library), false);
    let shown = CensusRows::from_flags(false, true);
    let report = census_report(library, false, &tally, &rows, shown).expect("a report");
    for name in names.split_whitespace() {
        let row = format!("  {name}: admitted");
        let lines = report.lines().filter(|line| line.starts_with(&row));
        assert_eq!(lines.count(), 1, "{library}: {name}");
    }
}
