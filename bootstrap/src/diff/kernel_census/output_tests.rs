//! ADR 4.10.26a AC3 — the two renderings and the exit code, split from
//! `tests.rs` by the file-size threshold.

use albite_bootstrap::elaborate::kernel::Cause;

use super::tests::parsed;
use super::*;

fn sample_diff() -> CensusDiff {
    let before = parsed(&[
        "z: named-type-reference / — — ty",
        "y: named-type-reference / — — ty",
        "x: hole / admitted — sorry",
        "w: hole / admitted — sorry",
        "gone: admitted / —",
    ]);
    let after = parsed(&[
        "z: admitted / —",
        "y: admitted / —",
        "x: unclassified / — — other",
        "w: hole / unclassified — sorry",
        "new: axiom / —",
    ]);
    join_reports(&before, &after)
}

// ADR 4.10.26a AC3 — cause order first, then names.
#[test]
fn the_text_render_is_ordered_by_cause_then_name() {
    let text = render_diff_text("before.txt", "after.txt", &sample_diff());
    let expected = "\
kernel census diff: before.txt -> after.txt
  moved:          3
  secondary only: 1
  only before:    1
  only after:     1

moves (primary cause):
  hole -> unclassified                                    1
  named-type-reference -> admitted                        2

hole -> unclassified:
  x

named-type-reference -> admitted:
  y
  z

secondary only:
  w: hole / admitted -> unclassified

only in before:
  gone

only in after:
  new
";
    assert_eq!(text, expected);
    let empty = render_diff_text("a", "b", &CensusDiff::default());
    assert!(empty.contains("moved:          0"));
    assert!(!empty.contains("moves (primary cause)"));
}

// ADR 4.10.26a AC3 — the `--json` object's shape and order.
#[test]
fn the_json_render_has_the_declared_shape() {
    let rendered = render_diff_json(&sample_diff());
    let expected = serde_json::json!({
        "moves": [
            { "from": "hole", "to": "unclassified", "count": 1, "names": ["x"] },
            { "from": "named-type-reference", "to": "admitted", "count": 2, "names": ["y", "z"] },
        ],
        "secondary_only": [{ "name": "w", "before": "admitted", "after": "unclassified" }],
        "only_before": ["gone"],
        "only_after": ["new"],
    });
    assert_eq!(rendered, expected);
    let none_secondary = SecondaryChange {
        name: "n".to_string(),
        primary: Cause::Hole,
        before: None,
        after: Some(Cause::Admitted),
    };
    let diff = CensusDiff {
        secondary_only: vec![none_secondary],
        ..CensusDiff::default()
    };
    assert_eq!(
        render_diff_json(&diff)["secondary_only"][0]["before"],
        serde_json::Value::Null
    );
}

// ADR 4.10.26a AC3 — unreadable and invalid both exit 2; a comparison exits 0.
#[test]
fn an_unreadable_file_and_an_invalid_report_both_exit_two() {
    let unreadable_failure = DiffFailure::Unreadable {
        path: "missing.txt".to_string(),
        reason: "No such file".to_string(),
    };
    let invalid_failure = DiffFailure::Invalid {
        path: "a.txt".to_string(),
        error: ReportError::MissingHeader,
    };
    assert_eq!(
        diff_report_texts(("a.txt", "garbage"), ("b.txt", "garbage")),
        Err(invalid_failure.clone())
    );
    assert_eq!(exit_code(&Err(unreadable_failure.clone())), 2);
    assert_eq!(exit_code(&Err(invalid_failure.clone())), 2);
    assert_eq!(exit_code(&Ok(sample_diff())), 0);
    assert_eq!(exit_code(&Ok(CensusDiff::default())), 0);
    let missing = std::path::Path::new("/nonexistent/kernel-census-before.txt");
    assert!(matches!(
        read_report(missing),
        Err(DiffFailure::Unreadable { .. })
    ));
    assert_eq!(
        unreadable_failure.to_string(),
        "error: cannot read missing.txt: No such file"
    );
    assert_eq!(
        invalid_failure.to_string(),
        "error: a.txt is not a complete kernel census report: no `kernel census: <file>` header"
    );
}
