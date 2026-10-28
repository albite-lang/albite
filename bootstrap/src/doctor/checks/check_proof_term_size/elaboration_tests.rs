//! `term-size` end to end (ADR 13.10.26c AC2–AC3, 15.10.26g AC1, D4, D8):
//! one cold elaboration of an inline fixture per test, through `census_file`,
//! the command entry point and the dispatcher. Split from `tests.rs`, which
//! keeps the pure count, population and render tests.

use std::path::PathBuf;
use std::process::ExitCode;

use super::tests::request;
use super::*;

const MAIN: &str = "pub mod lib;\n\
theorem four(): 4 == 4 { refl }\n\
theorem five(): 5 == 5 { refl }\n\
axiom trusted(n: Nat): Bool\n\
fn double(n: Nat) -> Nat { n + n }\n\
#[partial]\n\
fn spin(n: Nat) -> Nat { spin(n) }\n";

const LIB: &str = "theorem six(): 6 == 6 { refl }\n";

fn fixture() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = std::fs::canonicalize(dir.path()).expect("canonical");
    std::fs::create_dir_all(root.join("lib")).expect("mkdir");
    std::fs::write(root.join("main.tg"), MAIN).expect("write");
    std::fs::write(root.join("lib/mod.tg"), LIB).expect("write");
    let main = root.join("main.tg");
    (dir, main)
}

/// 13.10.26c AC2 / 15.10.26g AC1: one cold elaboration answers the file —
/// the theorems and the axiom by default, the `Total` function under
/// `--include-total`, the `#[partial]` one only under `--all-definitions` —
/// and no `.tungsten/` directory is left beside the fixture (D4).
#[test]
fn one_elaboration_answers_the_fixture() {
    let (_dir, main) = fixture();
    let rows = census_file(&main, Population::Proof).expect("elaborates");
    let named: Vec<(&str, &str, bool)> = rows
        .iter()
        .map(|r| (r.module.as_str(), r.name.as_str(), r.size.is_none()))
        .collect();
    assert_eq!(
        named,
        [
            ("lib", "six", false),
            ("main", "five", false),
            ("main", "four", false),
            ("main", "trusted", true),
        ]
    );
    assert!(rows[0].size.is_some_and(|s| s.refl == 1), "{rows:?}");

    let with_total = census_file(&main, Population::ProofTotal).expect("elaborates");
    let has = |rows: &[SizeRow], name: &str| rows.iter().any(|r| r.name == name);
    assert!(has(&with_total, "double"), "{with_total:?}");
    assert!(!has(&with_total, "spin"), "{with_total:?}");
    let all = census_file(&main, Population::All).expect("elaborates");
    assert!(has(&all, "spin") && has(&all, "double"), "{all:?}");
    assert!(all.len() > with_total.len() && with_total.len() > rows.len());

    let root = main.parent().expect("dir");
    assert!(!root.join(".albite").exists(), "the census wrote a cache");
    assert!(!root.join("lib/.tungsten").exists());
}

/// 13.10.26c AC2 (D5): an unelaboratable file and an empty population exit 2.
#[test]
fn an_unelaboratable_or_empty_file_exits_two() {
    let dir = tempfile::tempdir().expect("tempdir");
    let broken = dir.path().join("broken.tg");
    std::fs::write(&broken, "fn main() -> Nat { true }\n").expect("write");
    assert!(census_file(&broken, Population::Proof).is_err());
    let proof_json = request(Population::Proof, true);
    assert_eq!(
        cmd_check_proof_term_size(&broken, &proof_json),
        ExitCode::from(2)
    );

    let empty = dir.path().join("empty.tg");
    std::fs::write(&empty, "fn main() -> Nat { 0 }\n").expect("write");
    assert_eq!(census_file(&empty, Population::Proof), Ok(Vec::new()));
    let proof = request(Population::Proof, false);
    assert_eq!(cmd_check_proof_term_size(&empty, &proof), ExitCode::from(2));
    let total = request(Population::ProofTotal, false);
    assert_eq!(cmd_check_proof_term_size(&empty, &total), ExitCode::from(0));

    let missing = dir.path().join("missing.tg");
    assert!(census_file(&missing, Population::Proof).is_err());
}

/// 15.10.26g D8: `--json-out` writes the JSON object beside stdout's text,
/// and writes nothing when the census refuses.
#[test]
fn json_out_writes_the_report_only_when_it_answers() {
    let (dir, main) = fixture();
    let out = dir.path().join("census.json");
    let answered = TermSizeRequest {
        json_out: Some(out.clone()),
        ..request(Population::All, false)
    };
    assert_eq!(
        cmd_check_proof_term_size(&main, &answered),
        ExitCode::from(0)
    );
    let written = std::fs::read_to_string(&out).expect("written");
    let baseline = delta::parse_baseline(&written).expect("a census is a baseline");
    assert_eq!(baseline.population, "all");

    let refused_out = dir.path().join("refused.json");
    let refused = TermSizeRequest {
        json_out: Some(refused_out.clone()),
        ..request(Population::Proof, false)
    };
    let missing = dir.path().join("missing.tg");
    assert_eq!(
        cmd_check_proof_term_size(&missing, &refused),
        ExitCode::from(2)
    );
    assert!(!refused_out.exists());
}

/// 13.10.26c AC3: the dispatcher routes `term-size` and passes its exit through.
#[test]
fn the_dispatcher_runs_term_size() {
    use crate::doctor::check_commands::check_proof::{dispatch_check_proof, CheckProofCommands};
    let command = CheckProofCommands::TermSize {
        file: PathBuf::from("no/such/file.tg"),
        include_total: false,
        all_definitions: true,
        top: Some(3),
        baseline: None,
        max_growth: None,
        min_delta: None,
        json: true,
        json_out: None,
    };
    assert_eq!(dispatch_check_proof(command, false), ExitCode::from(2));
}
