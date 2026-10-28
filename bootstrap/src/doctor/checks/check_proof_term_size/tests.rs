//! `term-size` (ADR 13.10.26c, 15.10.26g): the count over hand-built terms,
//! and the population, `--top`, renderers and exit over injected rows. The
//! end-to-end fixture elaborations are in `elaboration_tests.rs`.

use std::path::Path;

use albite_core::terms::{SpannedTerm, TermSpan};
use albite_core::{Term, Type};
use serde_json::{json, Value};

use super::*;
use crate::span::Span;

pub(super) fn size(
    subst: usize,
    refl: usize,
    global_occurrences: usize,
    nodes: usize,
) -> ProofTermSize {
    ProofTermSize {
        subst,
        refl,
        global_occurrences,
        nodes,
        case_arms: 0,
    }
}

pub(super) fn request(population: Population, json: bool) -> TermSizeRequest {
    TermSizeRequest {
        population,
        json,
        top: None,
        baseline: None,
        thresholds: Thresholds::DEFAULT,
        json_out: None,
    }
}

/// 13.10.26c AC1: each counted kind is counted, and every node is a node.
#[test]
fn count_proof_nodes_counts_each_kind_and_every_node() {
    assert_eq!(count_proof_nodes(&Term::Zero), size(0, 0, 0, 1));
    assert_eq!(
        count_proof_nodes(&Term::refl(Type::Nat, Term::Zero)),
        size(0, 1, 0, 2)
    );
    assert_eq!(count_proof_nodes(&Term::global("f")), size(0, 0, 1, 1));
    let subst = Term::subst(Type::Nat, Type::Nat, Term::Zero, Term::Unit);
    assert_eq!(count_proof_nodes(&subst), size(1, 0, 0, 3));
}

/// 13.10.26c AC1: a `Subst` nested under a `Lambda` and an `App` is found,
/// and a `Global` cited twice counts twice.
#[test]
fn a_nested_subst_is_found_and_a_repeated_global_counts_twice() {
    let inner = Term::subst(
        Type::Nat,
        Type::Nat,
        Term::global("helper"),
        Term::refl(Type::Nat, Term::Zero),
    );
    let term = Term::lambda(
        "h",
        Type::Nat,
        Term::app(
            Term::global("helper"),
            Term::app(inner, Term::Var("h".into())),
        ),
    );
    // lambda, app, global, app, subst, global, refl, zero, var
    assert_eq!(count_proof_nodes(&term), size(1, 1, 2, 9));
}

/// 13.10.26c AC1 (D3): a `Global` in a `Subst`'s motive, or in a binder's
/// annotation, is in a `Type` and is never visited.
#[test]
fn the_types_a_term_carries_are_not_walked() {
    let mentions_global = Type::eq(Type::Nat, Term::global("g"), Term::global("g"));
    let subst = Term::subst(Type::Nat, mentions_global.clone(), Term::Zero, Term::Zero);
    assert_eq!(count_proof_nodes(&subst), size(1, 0, 0, 3));
    let binder = Term::lambda("h", mentions_global, Term::Zero);
    assert_eq!(count_proof_nodes(&binder), size(0, 0, 0, 2));
}

/// 15.10.26g AC1 (D3): 2 per `Case`, n per n-arm `AdtMatch`, nested ones
/// summed, and the other four counts as they were.
#[test]
fn case_arms_counts_two_per_case_and_n_per_adt_match() {
    let case = Term::case(Term::Zero, "l", Term::Zero, "r", Term::Unit);
    assert_eq!(
        count_proof_nodes(&case),
        ProofTermSize {
            case_arms: 2,
            ..size(0, 0, 0, 4)
        }
    );
    let arm = |tag: usize, body: Term| (tag, format!("x{tag}"), Box::new(body));
    let three = Term::adt_match(
        Term::Zero,
        vec![arm(0, Term::Zero), arm(1, case), arm(2, Term::global("g"))],
    );
    // match, scrutinee, three bodies, plus the nested case's three children
    assert_eq!(
        count_proof_nodes(&three),
        ProofTermSize {
            case_arms: 5,
            ..size(0, 0, 1, 8)
        }
    );
}

/// 13.10.26c D6: the totals sum field by field.
#[test]
fn sizes_sum_field_by_field() {
    let with_arms = |s: ProofTermSize, case_arms| ProofTermSize { case_arms, ..s };
    assert_eq!(
        with_arms(size(1, 2, 3, 4), 5).plus(with_arms(size(10, 20, 30, 40), 50)),
        with_arms(size(11, 22, 33, 44), 55)
    );
}

pub(super) fn row(
    module: &str,
    name: &str,
    proof: bool,
    counted: Option<ProofTermSize>,
) -> SizeRow {
    SizeRow {
        module: module.into(),
        name: name.into(),
        proof,
        size: counted,
    }
}

fn core_def(name: &str, term: Term, item_span: (u32, u32)) -> CoreDef {
    CoreDef {
        name: name.into(),
        ty: Type::Nat,
        term: SpannedTerm::new(term, TermSpan::new(item_span.0, item_span.1)),
        span: Span {
            start: item_span.0,
            end: item_span.1,
        },
    }
}

/// 13.10.26c D2 / 15.10.26g D2: each population's filter and the
/// `(module, name)` order, over injected candidates; an axiom has no counts.
#[test]
fn select_rows_keeps_each_population_in_module_then_name_order() {
    let axiom_body = Term::spanned(Term::Sorry, TermSpan::new(0, 9));
    let defs = [
        core_def("zeta", Term::refl(Type::Nat, Term::Zero), (0, 9)),
        core_def("alpha", Term::Zero, (0, 9)),
        core_def("choice", axiom_body, (0, 9)),
        core_def("helper", Term::Zero, (0, 9)),
        core_def("partial_fn", Term::Zero, (0, 9)),
    ];
    let candidate = |def, module: &str, proof, total| Candidate {
        module: module.into(),
        def,
        proof,
        total,
    };
    let candidates = [
        candidate(&defs[0], "main", true, true),
        candidate(&defs[1], "lib", true, false),
        candidate(&defs[2], "main", true, false),
        candidate(&defs[3], "main", false, true),
        candidate(&defs[4], "main", false, false),
    ];
    assert_eq!(
        select_rows(&candidates, Population::Proof),
        [
            row("lib", "alpha", true, Some(size(0, 0, 0, 1))),
            row("main", "choice", true, None),
            row("main", "zeta", true, Some(size(0, 1, 0, 2))),
        ]
    );
    let names = |population| -> Vec<String> {
        select_rows(&candidates, population)
            .into_iter()
            .map(|r| r.name)
            .collect()
    };
    assert_eq!(
        names(Population::ProofTotal),
        ["alpha", "choice", "helper", "zeta"]
    );
    assert_eq!(
        names(Population::All),
        ["alpha", "choice", "helper", "partial_fn", "zeta"]
    );
    assert!(!select_rows(&candidates, Population::ProofTotal)[2].proof);
}

/// 15.10.26g D2: the flags pick the population, and each has its label.
#[test]
fn population_follows_the_flags_and_labels_itself() {
    assert_eq!(Population::from_flags(false, false), Population::Proof);
    assert_eq!(Population::from_flags(true, false), Population::ProofTotal);
    assert_eq!(Population::from_flags(false, true), Population::All);
    let labels =
        [Population::Proof, Population::ProofTotal, Population::All].map(Population::label);
    assert_eq!(labels, ["proof", "proof+total", "all"]);
}

/// 13.10.26c D2: the root module's empty path names its file stem.
#[test]
fn a_root_module_is_named_by_its_file_stem() {
    let file = Path::new("dir/main.tg");
    assert_eq!(module_name(&[], file), "main");
    assert_eq!(
        module_name(&["nat".into(), "binom".into()], file),
        "nat::binom"
    );
}

fn three_rows() -> [SizeRow; 3] {
    [
        row("m", "t", true, Some(size(11, 1, 38, 412))),
        row("m", "u", false, Some(size(1, 2, 3, 4))),
        row("m", "ax", true, None),
    ]
}

/// 13.10.26c D6 / 15.10.26g D2, D3, D6: the JSON object, with the population,
/// `case_arms`, axiom nulls, non-axiom totals, and `"top"` only under `--top`.
#[test]
fn render_json_matches_d6() {
    let rows = three_rows();
    let parsed: Value =
        serde_json::from_str(&render_json("f.tg", Population::All, &rows, None)).expect("json");
    assert_eq!(
        parsed,
        json!({
            "file": "f.tg",
            "population": "all",
            "rows": [
                {"module": "m", "name": "t", "axiom": false, "subst": 11, "refl": 1,
                 "global_occurrences": 38, "nodes": 412, "case_arms": 0},
                {"module": "m", "name": "u", "axiom": false, "subst": 1, "refl": 2,
                 "global_occurrences": 3, "nodes": 4, "case_arms": 0},
                {"module": "m", "name": "ax", "axiom": true, "subst": null, "refl": null,
                 "global_occurrences": null, "nodes": null, "case_arms": null},
            ],
            "totals": {"definitions": 3, "axioms": 1, "subst": 12, "refl": 3,
                       "global_occurrences": 41, "nodes": 416, "case_arms": 0},
        })
    );
    let topped: Value =
        serde_json::from_str(&render_json("f.tg", Population::Proof, &rows, Some(1)))
            .expect("json");
    assert_eq!(topped["top"].as_array().map(Vec::len), Some(1));
    assert_eq!(topped["top"][0]["name"], "t");
    assert_eq!(topped["rows"].as_array().map(Vec::len), Some(3));
}

/// 13.10.26c §2 / 15.10.26g D6: the text form prints each row with its
/// `case-arms`, and `--top` keeps the largest while the totals cover all.
#[test]
fn render_text_prints_rows_and_totals() {
    let rows = three_rows();
    let text = render_text("f.tg", &rows, None);
    assert!(text.starts_with("proof term size: f.tg\n"), "{text}");
    assert!(text.contains("case-arms"), "{text}");
    assert!(
        text.contains(&format!(
            "  {:<32} {:<24} {:>6} {:>6} {:>10} {:>7} {:>9}\n",
            "t", "m", 11, 1, 38, 412, 0
        )),
        "{text}"
    );
    assert!(
        text.contains(&format!("  {:<32} {:<24}  axiom", "ax", "m")),
        "{text}"
    );
    let totals = "  3 definition(s), 2 proof-relevant, 1 axiom(s); totals: subst 12, refl 3, global-occ 41, nodes 416, case-arms 0\n";
    assert!(text.contains(totals), "{text}");

    let top = render_text("f.tg", &rows, Some(1));
    assert!(
        top.contains(&format!("  {:<32} {:<24} ", "t", "m")),
        "{top}"
    );
    assert!(
        !top.contains(&format!("  {:<32} {:<24} ", "u", "m")),
        "{top}"
    );
    assert!(top.contains(totals), "{top}");
}

/// 13.10.26c D5: a census exits 0 on stdout; an empty population and an
/// error exit 2, on stderr, with one `error` object under `--json`.
#[test]
fn term_size_output_follows_d5() {
    let rows = vec![row("m", "t", true, Some(size(0, 1, 0, 2)))];
    let text = request(Population::Proof, false);
    let as_json = request(Population::Proof, true);
    let (out, err, code) = term_size_output("f.tg", &text, Ok(rows.clone()), None);
    assert_eq!(
        (out, err, code),
        (render_text("f.tg", &rows, None), String::new(), 0)
    );
    let (out, _, code) = term_size_output("f.tg", &as_json, Ok(rows.clone()), None);
    assert_eq!(
        (out, code),
        (render_json("f.tg", Population::Proof, &rows, None), 0)
    );

    let (out, err, code) = term_size_output("f.tg", &text, Ok(Vec::new()), None);
    assert_eq!((out.as_str(), code), ("", 2));
    assert!(
        err.ends_with("not a clean one (try --include-total)\n"),
        "{err}"
    );
    let total = request(Population::ProofTotal, false);
    let (_, err, _) = term_size_output("f.tg", &total, Ok(Vec::new()), None);
    assert!(err.ends_with("(try --all-definitions)\n"), "{err}");
    let all = request(Population::All, false);
    let (_, err, _) = term_size_output("f.tg", &all, Ok(Vec::new()), None);
    assert!(err.ends_with("not a clean one\n"), "{err}");

    let (out, err, code) = term_size_output("f.tg", &as_json, Err("boom".into()), None);
    assert_eq!(code, 2);
    assert_eq!(err, "error: boom\n");
    let parsed: Value = serde_json::from_str(&out).expect("one object");
    assert_eq!(parsed, json!({"error": "boom"}));
}
