//! ADR 3.10.26p AC 1–3: records are presented by their encoding, exactly or
//! not at all, and the census and the gate present alike.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use albite_core::{Term, Type};

use super::{RecordPresentation, Unpresented};
use crate::doctor::checks::check_kernel_census::{record_presentation, run_census, Cause};
use crate::driver::{self, ProjectOutput};
use crate::elaborate::kernel::{
    decreasing_hints, term_verdicts, DefinitionSite, GateVerdict, KernelMode, Register, TermGate,
};
use crate::elaborate::termination::TerminationInput;

fn ty(name: &str) -> Type {
    Type::TyVar(name.to_string())
}

fn nat_pair() -> Type {
    Type::product(Type::Nat, Type::Nat)
}

/// A presentation over `records`, each `(name, Some(encoding))` or `(name,
/// None)` for a record with no encoding.
fn presentation(records: &[(&str, Option<Type>)]) -> RecordPresentation {
    let encodings: HashMap<String, Type> = records
        .iter()
        .filter_map(|(name, encoding)| Some((name.to_string(), encoding.clone()?)))
        .collect();
    let names: Vec<String> = records.iter().map(|(name, _)| name.to_string()).collect();
    RecordPresentation::new(&encodings, &names)
}

#[test]
fn a_record_is_presented_by_its_encoding() {
    let records = presentation(&[("Pt", Some(nat_pair()))]);
    assert_eq!(records.present_type(&ty("Pt")), nat_pair());
    let arrow = Type::arrow(ty("Pt"), Type::Nat);
    assert_eq!(
        records.present_type(&arrow),
        Type::arrow(nat_pair(), Type::Nat)
    );
    assert_eq!(records.presented_count(), 1);
}

#[test]
fn a_record_inside_a_sum_payload_is_presented() {
    let records = presentation(&[("Pt", Some(nat_pair()))]);
    let wrap = Type::sum(ty("Pt"), Type::Unit);
    assert_eq!(
        records.present_type(&wrap),
        Type::sum(nat_pair(), Type::Unit)
    );
}

#[test]
fn a_record_naming_a_record_is_presented_to_a_fixed_point() {
    let records = presentation(&[
        ("Line", Some(Type::product(ty("Pt"), ty("Pt")))),
        ("Pt", Some(nat_pair())),
    ]);
    let expected = Type::product(nat_pair(), nat_pair());
    assert_eq!(records.present_type(&ty("Line")), expected);
    assert_eq!(records.presented("Line"), Some(&expected));
}

#[test]
fn a_record_cycle_terminates() {
    // A self-recursive record spelled by name, as the elaborator encodes
    // `type Chain = { v: Nat, next: Tail }` with `Tail = Stop | More(Chain)`.
    let chain = Type::product(Type::Nat, Type::sum(Type::Unit, ty("Chain")));
    // A two-record cycle, and a record that only reaches into it.
    let records = presentation(&[
        ("Chain", Some(chain)),
        ("A", Some(Type::product(Type::Nat, ty("B")))),
        ("B", Some(Type::product(Type::Bool, ty("A")))),
        ("C", Some(Type::product(Type::Nat, ty("A")))),
    ]);
    for name in ["Chain", "A", "B"] {
        assert_eq!(
            records.unpresented(name),
            Some(Unpresented::Cycle),
            "{name}"
        );
        assert_eq!(records.present_type(&ty(name)), ty(name), "{name}");
    }
    assert_eq!(
        records.present_type(&ty("C")),
        Type::product(Type::Nat, ty("A")),
        "a record outside the cycle is presented, the cycle left named inside it"
    );
}

#[test]
fn a_self_recursive_record_through_a_sum_is_presented() {
    let bound = Type::mu(
        "α_R",
        Type::product(Type::Nat, Type::sum(Type::Unit, ty("α_R"))),
    );
    let records = presentation(&[("R", Some(bound.clone()))]);
    assert_eq!(records.unpresented("R"), None);
    assert_eq!(records.present_type(&ty("R")), bound);
}

#[test]
fn a_type_parameter_spelled_like_no_record_is_untouched() {
    let records = presentation(&[("Pt", Some(nat_pair()))]);
    assert_eq!(records.present_type(&ty("T")), ty("T"));
    // A binder spelled like a record shadows it.
    let shadowed = Type::forall("Pt", Type::arrow(ty("Pt"), ty("Pt")));
    assert_eq!(records.present_type(&shadowed), shadowed);
}

#[test]
fn an_at_prefixed_name_is_not_a_record_key() {
    let records = presentation(&[("Pt", Some(nat_pair()))]);
    assert_eq!(records.present_type(&ty("@Pt")), ty("@Pt"));
    let at_keyed = presentation(&[("@Pt", Some(nat_pair()))]);
    assert_eq!(at_keyed.present_type(&ty("Pt")), ty("Pt"));
}

#[test]
fn a_cluster_member_with_a_free_marker_is_left_named() {
    // A non-entry member: its encoding names the entry's marker, unbound.
    let member = Type::product(Type::Nat, ty("α_Forest"));
    let records = presentation(&[("Rose", Some(member))]);
    assert_eq!(records.unpresented("Rose"), Some(Unpresented::FreeMarker));
    assert_eq!(records.present_type(&ty("Rose")), ty("Rose"));
}

/// P0.2's rule: a generic record holds no encoding (its instances arrive
/// structural), and an encoding open over a parameter is never presented.
#[test]
fn a_generic_record_follows_p0s_rule() {
    let records = presentation(&[("Box", None), ("Open", Some(ty("T")))]);
    assert_eq!(records.unpresented("Box"), Some(Unpresented::NoEncoding));
    assert_eq!(records.unpresented("Open"), Some(Unpresented::OpenEncoding));
    assert_eq!(records.present_type(&ty("Box")), ty("Box"));
    assert_eq!(records.present_type(&ty("Open")), ty("Open"));
    let counts = records.unpresented_counts();
    assert_eq!(counts.get(&Unpresented::NoEncoding), Some(&1));
    assert_eq!(counts.get(&Unpresented::OpenEncoding), Some(&1));
}

#[test]
fn presentation_is_idempotent() {
    let chain = Type::product(Type::Nat, Type::sum(Type::Unit, ty("Chain")));
    let records = presentation(&[
        ("Chain", Some(chain)),
        ("Line", Some(Type::product(ty("Pt"), ty("Pt")))),
        ("Pt", Some(nat_pair())),
    ]);
    let mixed = Type::arrow(
        Type::sum(ty("Line"), ty("Chain")),
        Type::eq(ty("Pt"), Term::var("p"), Term::var("p")),
    );
    let once = records.present_type(&mixed);
    assert_ne!(once, mixed);
    assert_eq!(records.present_type(&once), once);
    let term = Term::lambda("p", ty("Line"), Term::refl(ty("Pt"), Term::var("p")));
    let presented = records.present_term(&term);
    assert_eq!(
        presented,
        Term::lambda(
            "p",
            Type::product(nat_pair(), nat_pair()),
            Term::refl(nat_pair(), Term::var("p")),
        )
    );
    assert_eq!(records.present_term(&presented), presented);
}

#[test]
fn an_empty_presentation_presents_nothing() {
    let records = RecordPresentation::EMPTY;
    assert!(records.is_empty());
    assert!(!presentation(&[("Pt", Some(nat_pair()))]).is_empty());
    assert_eq!(records.present_type(&ty("Pt")), ty("Pt"));
    assert_eq!(records.unpresented_counts(), BTreeMap::new());
}

fn type_nodes(ty: &Type) -> usize {
    1 + ty.children().into_iter().map(type_nodes).sum::<usize>()
}

/// D2's size note and D4's numerator: how many of `main.tg`'s records are
/// presented, how many are left named and why, and the largest presentation.
/// Ignored — it elaborates the self-hosted compiler; run it with
/// `--ignored --nocapture`.
#[test]
#[ignore = "elaborates src/compiler/main.tg; a measurement, run by hand"]
fn d4_counts_main_tgs_records() {
    let main = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/compiler/main.tg");
    let reporting = crate::elaborate::termination::ReportingOnly::begin();
    let project = driver::elaborate_project_cold(&main, 20).expect("main.tg elaborates");
    drop(reporting);
    let started = std::time::Instant::now();
    let records = record_presentation(&project);
    let built = started.elapsed();
    let largest = project
        .record_types
        .keys()
        .filter_map(|name| Some((type_nodes(records.presented(name)?), name)))
        .max();
    println!(
        "records: {} · presented: {} · left named: {:?} · built in {built:?} · largest: {largest:?}",
        project.record_types.len(),
        records.presented_count(),
        records.unpresented_counts(),
    );
    let mut named: Vec<(&String, Unpresented)> = project
        .record_types
        .keys()
        .filter_map(|name| Some((name, records.unpresented(name)?)))
        .collect();
    named.sort();
    for (name, why) in named {
        println!("  {name}: {why:?}");
    }
}

// ── AC 2 — twins ─────────────────────────────────────────────────────────

/// 2.10.26l D6: a record is presented by the encoding the presentation was
/// built from, and by no other, so two shapes under one name never meet.
#[test]
fn records_of_different_shapes_never_meet() {
    let bool_then_nat = Type::product(Type::Bool, Type::Nat);
    let module_a = presentation(&[("Pt", Some(nat_pair()))]);
    let module_b = presentation(&[("Pt", Some(bool_then_nat.clone()))]);
    assert_eq!(module_a.present_type(&ty("Pt")), nat_pair());
    assert_eq!(module_b.present_type(&ty("Pt")), bool_then_nat);
}

/// P0.2's finding: two modules declaring a record of one name are rejected
/// before the kernel (the later declaration wins Signature Collection, so the
/// earlier module's field access is E0052) — never presented by one encoding.
#[test]
fn same_named_records_from_two_modules_are_not_conflated() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), "a.tg", "pub type Pt = { x: Nat, y: Nat }\npub theorem a_fst(p: Pt) -> Eq<Nat, p.x, p.x> { refl }\n");
    write(dir.path(), "b.tg", "pub type Pt = { flag: Bool, n: Nat }\npub theorem b_fst(p: Pt) -> Eq<Bool, p.flag, p.flag> { refl }\n");
    let main = write(
        dir.path(),
        "main.tg",
        "mod a;\nmod b;\nuse a::{Pt, a_fst};\nuse b::{b_fst};\nfn main() -> Nat { 0 }\n",
    );
    let error = driver::elaborate_project_cold(&main, 20)
        .err()
        .expect("same-named records are rejected before the kernel");
    let message = error.to_string();
    assert!(!message.contains("E0081"), "{message}");
}

// ── AC 1 (last) and AC 3 — the census and the gate, end to end ───────────

const RECORD_THEOREMS: &str = "\
type Pt = { x: Nat, y: Nat }
type Line = { a: Pt, b: Pt }
fn mk() -> Pt { Pt { x: 0, y: 1 } }
theorem pt_refl(p: Pt) -> Eq<Pt, p, p> { refl }
theorem line_refl(l: Line) -> Eq<Line, l, l> { refl }
theorem mk_refl() -> Eq<Pt, mk(), mk()> { refl }
fn main() -> Nat { 0 }
";

fn write(dir: &Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, source).expect("write fixture");
    path
}

fn elaborate(source: &str) -> (tempfile::TempDir, ProjectOutput) {
    let dir = tempfile::tempdir().expect("tempdir");
    let main = write(dir.path(), "main.tg", source);
    // Enforcement: the build's own admission gate runs here, and a refusal is
    // a hard error (E0081) — so `Ok` is the gate passing every theorem.
    let project = driver::elaborate_project_cold(&main, 20).expect("the fixture builds");
    (dir, project)
}

/// 3.10.26p AC 3: `pt_refl`, refused `named-type-reference` before this ADR,
/// is admitted by the census and passes enforcement.
#[test]
fn a_record_theorem_is_admitted() {
    let (_dir, project) = elaborate(RECORD_THEOREMS);
    let (tally, rows) = run_census(&project, false);
    for name in ["pt_refl", "line_refl", "mk_refl"] {
        let row = rows.iter().find(|row| row.name == name).expect("a row");
        assert_eq!(row.primary, Cause::Admitted, "{name}: {:?}", row.refusal);
    }
    assert_eq!(tally.refused, 0);
}

/// The census and the gate build their contexts from one presentation of one
/// project, so they decide every theorem alike — and the bound global `mk`
/// is presented in both, or `mk_refl` would be refused by one of them.
#[test]
fn the_census_and_the_gate_present_alike() {
    let (dir, project) = elaborate(RECORD_THEOREMS);
    let census = record_presentation(&project);
    let gate_side = RecordPresentation::new(&project.encoded_types, project.record_types.keys());
    for name in ["Pt", "Line"] {
        assert_eq!(census.presented(name), gate_side.presented(name), "{name}");
        assert!(census.presented(name).is_some(), "{name}");
    }
    let (_, rows) = run_census(&project, false);
    let admission = TerminationInput::from_meta(&project.termination_meta)
        .check(&project.defs)
        .admission;
    let hints = decreasing_hints(&project.termination_meta);
    let module = dir.path().join("main.tg");
    let sites: Vec<DefinitionSite<'_>> = project
        .defs
        .iter()
        .map(|def| DefinitionSite {
            module: Some(&module),
            def,
        })
        .collect();
    let gate = TermGate {
        mode: KernelMode::Gate,
        register: &Register::EMPTY,
        root: None,
        concrete_type_names: None,
        decreasing: Some(&hints),
        presentation: &gate_side,
    };
    let theorems: Vec<_> = ["pt_refl", "line_refl", "mk_refl"]
        .iter()
        .map(|name| {
            let def = project
                .defs
                .iter()
                .find(|def| def.name == *name)
                .expect("def");
            (def, module.as_path())
        })
        .collect();
    let verdicts = term_verdicts(&sites, &admission, &gate, &theorems);
    for ((def, _), verdict) in theorems.iter().zip(verdicts) {
        let row = rows.iter().find(|row| row.name == def.name).expect("a row");
        assert_eq!(verdict, GateVerdict::Pass, "{}", def.name);
        assert_eq!(row.primary, Cause::Admitted, "{}", def.name);
    }
    // Without the presentation the gate refuses the same theorem.
    let unpresented = TermGate {
        presentation: &RecordPresentation::EMPTY,
        ..gate
    };
    let refused = term_verdicts(&sites, &admission, &unpresented, &theorems[..1]);
    assert_ne!(refused[0], GateVerdict::Pass);
}
