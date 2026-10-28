//! ADR 2.10.26l AC 4: the census binds admitted bodies exactly as the gate
//! does (`elaborate::kernel::bind_with_bodies`), so the δ goldens are
//! admitted and their register rows are gone, while a `#[partial]` body — or
//! one tainted by it — is never unfolded.

use super::*;
use albite_core::conversion::Unfoldings;

use crate::elaborate::kernel::{bind_with_bodies, delta_admission, Register};

const DELTA_GOLDEN: &str = "tests/golden/check/conversion_unfolds_admitted_globals.tg";
const PARTIAL_GOLDEN: &str = "tests/golden/error/kernel_delta_never_unfolds_partial.tg";

fn repo_path(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(relative)
}

#[test]
fn census_admits_the_delta_goldens() {
    let project = crate::driver::elaborate_project(&repo_path(DELTA_GOLDEN), false, 20, None)
        .expect("the golden elaborates");
    let (tally, rows) = run_census(&project, true);
    // The three former register rows, `length_one` (D8's non-theorem row) and
    // `length` itself, whose self-call the kernel now presents as a `fix`.
    for name in [
        "two_is_two",
        "length_two",
        "via_two",
        "length_one",
        "length",
    ] {
        let row = rows
            .iter()
            .find(|row| row.name == name)
            .unwrap_or_else(|| panic!("{name} has a census row"));
        assert_eq!(row.primary, Cause::Admitted, "{name}: {:?}", row.refusal);
    }
    assert_eq!(tally.refused, 0);
    let registered: Vec<&str> = Register::real()
        .rows()
        .iter()
        .filter(|row| row.file == DELTA_GOLDEN)
        .map(|row| row.theorem.as_str())
        .collect();
    assert!(registered.is_empty(), "{registered:?}");
}

/// ADR 2.10.26l A4, D4's measurement: how many of `main.tg`'s bodies the
/// kernel admits, and how many of them as a re-checked `fix`. Ignored — it
/// elaborates the self-hosted compiler; run it with `--ignored --nocapture`.
#[test]
#[ignore = "elaborates src/compiler/main.tg; a measurement, run by hand"]
fn d4_counts_the_fixpoints_main_tg_admits() {
    let reporting = crate::elaborate::termination::ReportingOnly::begin();
    let project =
        crate::driver::elaborate_project(&repo_path("src/compiler/main.tg"), false, 20, None)
            .expect("main.tg elaborates");
    drop(reporting);
    let admission =
        crate::elaborate::termination::TerminationInput::from_meta(&project.termination_meta)
            .check(&project.defs)
            .admission;
    let hints = crate::elaborate::kernel::decreasing_hints(&project.termination_meta);
    let started = std::time::Instant::now();
    let admitted = delta_admission(&project.defs, &admission, Some(&hints)).admit_all();
    let names = admitted.names();
    let fixpoints: Vec<&&str> = names
        .iter()
        .filter(|name| admitted.decreasing(name).is_some())
        .collect();
    println!(
        "admitted {} bodies, {} as fix, in {:?}",
        admitted.names().len(),
        fixpoints.len(),
        started.elapsed()
    );
    println!("fix: {fixpoints:?}");
    assert!(fixpoints.len() <= admitted.names().len());
}

/// `name : Nat := body`, at an arbitrary span.
fn nat_def(name: &str, body: Term) -> CoreDef {
    CoreDef {
        name: name.into(),
        ty: Type::Nat,
        term: SpannedTerm::new(body, TermSpan::new(0, 1)),
        span: Span::new(0, 1),
    }
}

/// The golden twin cannot reach the census: the elaborator refuses both of
/// its theorems first (E0063, and E0071 because its own δ will not open a
/// partial body either). So the census's own binder is run over the same
/// shapes by hand — `two` `#[partial]`, `wrapped_two` tainted by it, both
/// `Partial` in the report — and each theorem stays refused.
#[test]
fn census_keeps_partial_out_of_delta() {
    let reporting = crate::elaborate::termination::ReportingOnly::begin();
    let elaborated = crate::driver::elaborate_project(&repo_path(PARTIAL_GOLDEN), false, 20, None);
    drop(reporting);
    assert!(elaborated.is_err(), "the elaborator refuses the twin first");

    let defs = [
        nat_def("two", Term::NatLit(2)),
        nat_def("wrapped_two", Term::global("two")),
    ];
    let admission = BTreeMap::from([
        ("two".to_string(), AdmissionState::Partial),
        ("wrapped_two".to_string(), AdmissionState::Partial),
    ]);
    let defs_by_name: HashMap<&str, &Type> =
        defs.iter().map(|d| (d.name.as_str(), &d.ty)).collect();
    let mut delta = delta_admission(&defs, &admission, None);
    for cited in ["two", "wrapped_two"] {
        let mentioned = BTreeSet::from([cited.to_string()]);
        let ctx = bind_with_bodies(&mentioned, &defs_by_name, &admission, &mut delta);
        let statement = eq_nat(Term::global(cited), Term::NatLit(2));
        let proof = Term::refl(Type::Nat, Term::global(cited));
        assert!(check_proof(&ctx, &proof, &statement).is_err(), "{cited}");
    }
    // The same shapes, `Total`, are unfolded: the refusal above is the
    // report's verdict at work, not a binder that never binds.
    let total = BTreeMap::from([
        ("two".to_string(), AdmissionState::Total),
        ("wrapped_two".to_string(), AdmissionState::Total),
    ]);
    let mut delta = delta_admission(&defs, &total, None);
    let mentioned = BTreeSet::from(["wrapped_two".to_string()]);
    let ctx = bind_with_bodies(&mentioned, &defs_by_name, &total, &mut delta);
    let statement = eq_nat(Term::global("wrapped_two"), Term::NatLit(2));
    let proof = Term::refl(Type::Nat, Term::global("wrapped_two"));
    assert_eq!(check_proof(&ctx, &proof, &statement), Ok(()));
}
