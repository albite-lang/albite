//! Tests for `albite info eval externs`.

use albite_core::eval::extern_registry::EXECUTABLE_EXTERNS;

use super::{cmd_info_eval_externs, render_human, render_json};

/// Every registered extern appears in the human table — the command's whole
/// job is to be exhaustive, so a truncating renderer is the failure to catch.
#[test]
fn human_output_lists_every_registered_extern() {
    let rendered = render_human(EXECUTABLE_EXTERNS);
    for entry in EXECUTABLE_EXTERNS {
        assert!(
            rendered.contains(entry.name),
            "{} missing from the rendered table",
            entry.name
        );
        assert!(
            rendered.contains(entry.summary),
            "{}'s summary missing from the rendered table",
            entry.name
        );
    }
}

/// The table states the consequence of absence, since that is the whole point:
/// a reader who does not know the failure is silent cannot use this list.
#[test]
fn human_output_explains_the_silent_failure() {
    let rendered = render_human(EXECUTABLE_EXTERNS);
    assert!(rendered.contains("Stuck"));
    assert!(
        rendered.contains("extern-coverage"),
        "should point at the per-file check"
    );
}

/// The count line matches the table — a mismatch would mislead about coverage.
#[test]
fn human_output_reports_the_registry_size() {
    let rendered = render_human(EXECUTABLE_EXTERNS);
    assert!(rendered.contains(&format!(
        "{} extern(s) registered",
        EXECUTABLE_EXTERNS.len()
    )));
}

/// Names are column-aligned, so the kind/summary columns stay readable.
#[test]
fn human_output_aligns_the_name_column() {
    let rendered = render_human(EXECUTABLE_EXTERNS);
    let kind_columns: Vec<usize> = rendered
        .lines()
        .filter(|l| l.starts_with("  tg_"))
        .filter_map(|l| l.find('['))
        .collect();
    assert!(!kind_columns.is_empty(), "expected rendered extern rows");
    assert!(
        kind_columns.iter().all(|c| *c == kind_columns[0]),
        "kind column should be aligned, got offsets {kind_columns:?}"
    );
}

/// An empty registry renders without panicking (the `max()` has no elements) —
/// a defensive case the real registry never hits but the renderer must survive.
#[test]
fn human_output_survives_an_empty_registry() {
    let rendered = render_human(&[]);
    assert!(rendered.contains("0 extern(s) registered"));
}

/// The JSON form carries one object per extern with all three fields.
#[test]
fn json_output_carries_every_field() {
    let rendered = render_json(EXECUTABLE_EXTERNS);
    assert_eq!(
        rendered.matches("\"name\"").count(),
        EXECUTABLE_EXTERNS.len()
    );
    for entry in EXECUTABLE_EXTERNS {
        assert!(rendered.contains(&format!("\"name\": \"{}\"", entry.name)));
    }
    assert!(rendered.starts_with('['));
    assert!(rendered.trim_end().ends_with(']'));
}

/// The JSON is actually parseable — a hand-rolled serializer is exactly the
/// thing that silently emits invalid output.
#[test]
fn json_output_parses() {
    let rendered = render_json(EXECUTABLE_EXTERNS);
    let parsed: serde_json::Value =
        serde_json::from_str(&rendered).expect("render_json must emit valid JSON");
    assert_eq!(
        parsed.as_array().map(Vec::len),
        Some(EXECUTABLE_EXTERNS.len())
    );
}

/// The command succeeds in both output modes.
#[test]
fn the_command_succeeds() {
    assert_eq!(
        cmd_info_eval_externs(false),
        std::process::ExitCode::SUCCESS
    );
    assert_eq!(cmd_info_eval_externs(true), std::process::ExitCode::SUCCESS);
}

// 4.10.26n AC 5, 6.10.26n AC 2: eight byte intrinsics are listed, and the
// seventh and eighth are marked reserved in both forms; 9.10.26n D2: so is
// the ninth.
#[test]
fn nine_byte_intrinsics_are_listed_and_three_are_reserved() {
    let bytes = EXECUTABLE_EXTERNS
        .iter()
        .filter(|entry| entry.name.starts_with("tg_bytes_"))
        .count();
    assert_eq!(bytes, 9);
    let human = render_human(EXECUTABLE_EXTERNS);
    let reserved: Vec<&str> = human.lines().filter(|l| l.contains("] reserved")).collect();
    assert_eq!(reserved.len(), 3, "{human}");
    assert!(reserved[2].contains("tg_bytes_new_uninit"), "{human}");
    assert!(
        reserved[0].contains("tg_bytes_into_string_unchecked"),
        "{human}"
    );
    assert!(
        reserved[1].contains("tg_bytes_from_string_grown"),
        "{human}"
    );
    let json = render_json(EXECUTABLE_EXTERNS);
    assert_eq!(json.matches("\"reserved\": true").count(), 3, "{json}");
    assert_eq!(
        json.matches("\"reserved\": false").count(),
        EXECUTABLE_EXTERNS.len() - 3
    );
}
