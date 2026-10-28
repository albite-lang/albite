//! The JUnit document builder over hand-built records (ADR 30.9.26k AC 3).
//! Every rendered document is parsed back by `roxmltree`.

use super::{classname, document, seconds, stop_document, StopKind, SuiteInfo};
use crate::test_runner::json::{ExpectedFailureRecord, TestRecord};

fn record(name: &str, verdict: &'static str, gating: bool) -> TestRecord {
    TestRecord {
        schema: 1,
        kind: "test",
        name: name.to_string(),
        module: None,
        file: "dir/entry.tg".to_string(),
        verdict,
        gating,
        message: None,
        assertions: 1,
        duration_ms: 12,
        expected_failure: None,
        stdout: String::new(),
        stderr: String::new(),
    }
}

fn suite(duration_ms: u64) -> SuiteInfo {
    SuiteInfo::for_entry("dir/entry.tg", duration_ms)
}

/// Parse `xml`, panicking with the document on a well-formedness failure.
fn parse(xml: &str) -> roxmltree::Document<'_> {
    roxmltree::Document::parse(xml).unwrap_or_else(|e| panic!("{e}:\n{xml}"))
}

fn testcases<'a>(doc: &'a roxmltree::Document<'a>) -> Vec<roxmltree::Node<'a, 'a>> {
    doc.descendants()
        .filter(|n| n.has_tag_name("testcase"))
        .collect()
}

fn child<'a>(node: roxmltree::Node<'a, 'a>, tag: &str) -> Option<roxmltree::Node<'a, 'a>> {
    node.children().find(|n| n.has_tag_name(tag))
}

/// `(tests, failures, errors, skipped, time)` on `node`.
fn counts(node: roxmltree::Node<'_, '_>) -> [String; 5] {
    ["tests", "failures", "errors", "skipped", "time"]
        .map(|a| node.attribute(a).unwrap().to_string())
}

const GATING_VERDICTS: [&str; 6] = [
    "failed",
    "timed_out",
    "black_hole",
    "never_compared",
    "asserted_nothing",
    "did_not_finish",
];

// 30.9.26k AC3: every gating verdict is a `<failure>` typed by the verdict.
#[test]
fn each_gating_verdict_is_a_failure_of_its_type() {
    for verdict in GATING_VERDICTS {
        let mut rec = record("t", verdict, true);
        rec.message = Some(format!("{verdict} happened"));
        let xml = document(&[rec], &suite(0));
        let doc = parse(&xml);
        let failure = child(testcases(&doc)[0], "failure").expect(verdict);
        assert_eq!(failure.attribute("type"), Some(verdict));
        assert_eq!(
            failure.attribute("message"),
            Some(format!("{verdict} happened").as_str())
        );
    }
}

#[test]
fn a_failure_without_a_message_takes_the_verdict() {
    let xml = document(&[record("t", "asserted_nothing", true)], &suite(0));
    let doc = parse(&xml);
    let failure = child(testcases(&doc)[0], "failure").unwrap();
    assert_eq!(failure.attribute("message"), Some("asserted_nothing"));
}

#[test]
fn a_failure_carries_the_diff_and_no_system_err() {
    let mut rec = record("t", "failed", true);
    rec.stderr = "  left:  2\n  right: 3\n".to_string();
    let xml = document(&[rec], &suite(0));
    let doc = parse(&xml);
    let case = testcases(&doc)[0];
    assert_eq!(
        child(case, "failure").unwrap().text(),
        Some("  left:  2\n  right: 3\n")
    );
    assert!(child(case, "system-err").is_none());
}

#[test]
fn a_passing_test_has_no_child_and_its_stderr_is_system_err() {
    let bare = document(&[record("t", "passed", false)], &suite(0));
    let doc = parse(&bare);
    assert_eq!(testcases(&doc)[0].children().count(), 0);

    let mut rec = record("t", "passed", false);
    rec.stderr = "warn".to_string();
    let xml = document(&[rec], &suite(0));
    let doc = parse(&xml);
    let case = testcases(&doc)[0];
    assert!(child(case, "failure").is_none());
    assert!(child(case, "skipped").is_none());
    assert_eq!(child(case, "system-err").unwrap().text(), Some("warn"));
}

#[test]
fn system_out_appears_only_when_stdout_is_non_empty() {
    let mut printing = record("p", "passed", false);
    printing.stdout = "hello <from> body\n".to_string();
    let xml = document(&[printing, record("q", "passed", false)], &suite(0));
    let doc = parse(&xml);
    let cases = testcases(&doc);
    assert_eq!(
        child(cases[0], "system-out").unwrap().text(),
        Some("hello <from> body\n")
    );
    assert!(child(cases[1], "system-out").is_none());
}

#[test]
fn skipped_is_skipped_with_its_message() {
    let mut rec = record("t", "skipped", false);
    rec.message = Some("check-only".to_string());
    let xml = document(&[rec], &suite(0));
    let doc = parse(&xml);
    let skipped = child(testcases(&doc)[0], "skipped").unwrap();
    assert_eq!(skipped.attribute("message"), Some("check-only"));
}

#[test]
fn an_expected_failure_is_skipped_naming_its_verdict_and_owner() {
    let mut rec = record("t", "expected_failure", false);
    rec.expected_failure = Some(ExpectedFailureRecord {
        owner: "9.9.26z".to_string(),
        reported: "failed",
    });
    rec.stderr = "diff".to_string();
    let xml = document(&[rec], &suite(0));
    let doc = parse(&xml);
    let case = testcases(&doc)[0];
    assert_eq!(
        child(case, "skipped").unwrap().attribute("message"),
        Some("expected failure: failed (owned by 9.9.26z)")
    );
    assert!(child(case, "failure").is_none());
    assert_eq!(child(case, "system-err").unwrap().text(), Some("diff"));
}

#[test]
fn every_testcase_carries_exactly_the_five_attributes_in_order() {
    let records: Vec<TestRecord> = GATING_VERDICTS
        .iter()
        .map(|v| record("t", v, true))
        .chain([record("p", "passed", false), record("s", "skipped", false)])
        .collect();
    let xml = document(&records, &suite(0));
    let doc = parse(&xml);
    for case in testcases(&doc) {
        let names: Vec<&str> = case.attributes().map(|a| a.name()).collect();
        assert_eq!(names, ["name", "classname", "file", "assertions", "time"]);
    }
    let stop = stop_document(&[], StopKind::Runner, "boom", &suite(0));
    let doc = parse(&stop);
    let names: Vec<&str> = testcases(&doc)[0].attributes().map(|a| a.name()).collect();
    assert_eq!(names, ["name", "classname", "file", "assertions", "time"]);
}

#[test]
fn testcase_attributes_carry_the_record() {
    let mut rec = record("test_x", "passed", false);
    rec.assertions = 7;
    rec.duration_ms = 1500;
    let xml = document(&[rec], &suite(0));
    let doc = parse(&xml);
    let case = testcases(&doc)[0];
    assert_eq!(case.attribute("name"), Some("test_x"));
    assert_eq!(case.attribute("classname"), Some("entry"));
    assert_eq!(case.attribute("file"), Some("dir/entry.tg"));
    assert_eq!(case.attribute("assertions"), Some("7"));
    assert_eq!(case.attribute("time"), Some("1.500"));
}

#[test]
fn counts_are_derived_from_the_records_on_both_roots() {
    let mut expected = record("e", "expected_failure", false);
    expected.expected_failure = Some(ExpectedFailureRecord {
        owner: "o".to_string(),
        reported: "failed",
    });
    let records = vec![
        record("a", "passed", false),
        record("b", "failed", true),
        record("c", "timed_out", true),
        record("d", "skipped", false),
        expected,
    ];
    let xml = document(&records, &suite(2345));
    let doc = parse(&xml);
    let want = ["5", "2", "0", "2", "2.345"].map(String::from);
    let root = doc.root_element();
    assert_eq!(root.attribute("name"), Some("albite test"));
    assert_eq!(counts(root), want);
    let testsuite = child(root, "testsuite").unwrap();
    assert_eq!(testsuite.attribute("name"), Some("dir/entry.tg"));
    assert_eq!(counts(testsuite), want);
}

#[test]
fn suite_time_is_the_elapsed_time_not_the_sum() {
    let xml = document(
        &[record("a", "passed", false), record("b", "passed", false)],
        &suite(5),
    );
    let doc = parse(&xml);
    assert_eq!(doc.root_element().attribute("time"), Some("0.005"));
}

#[test]
fn an_empty_run_has_zero_tests_and_no_testcase() {
    let xml = document(&[], &suite(0));
    let doc = parse(&xml);
    assert_eq!(
        counts(doc.root_element()),
        ["0", "0", "0", "0", "0.000"].map(String::from)
    );
    assert!(testcases(&doc).is_empty());
}

#[test]
fn classname_dots_the_module_path_and_uses_the_stem_for_the_entry() {
    let mut rec = record("t", "passed", false);
    assert_eq!(classname(&rec, "entry"), "entry");
    rec.module = Some("a::b".to_string());
    assert_eq!(classname(&rec, "entry"), "a.b");
    let doc_xml = document(&[rec], &suite(0));
    let doc = parse(&doc_xml);
    assert_eq!(testcases(&doc)[0].attribute("classname"), Some("a.b"));
}

#[test]
fn suite_info_takes_the_stem_of_the_entry_file() {
    let info = SuiteInfo::for_entry("tests/test_runner_pass_fail.tg", 3);
    assert_eq!(info.name, "tests/test_runner_pass_fail.tg");
    assert_eq!(info.entry_stem, "test_runner_pass_fail");
    assert_eq!(info.duration_ms, 3);
}

#[test]
fn seconds_has_exactly_three_decimals() {
    assert_eq!(seconds(0), "0.000");
    assert_eq!(seconds(12), "0.012");
    assert_eq!(seconds(1500), "1.500");
    assert_eq!(seconds(61_001), "61.001");
}

#[test]
fn a_stop_document_is_one_error_testcase_at_time_zero() {
    for (kind, name, error_type) in [
        (StopKind::Runner, "(runner)", "runner_error"),
        (StopKind::Diagnostics, "(elaboration)", "diagnostics"),
    ] {
        let xml = stop_document(&[], kind, "it <broke>", &suite(999));
        let doc = parse(&xml);
        let root = doc.root_element();
        assert_eq!(
            counts(root),
            ["1", "0", "1", "0", "0.000"].map(String::from)
        );
        let cases = testcases(&doc);
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].attribute("name"), Some(name));
        assert_eq!(cases[0].attribute("classname"), Some("entry"));
        let error = child(cases[0], "error").unwrap();
        assert_eq!(error.attribute("type"), Some(error_type));
        assert_eq!(error.attribute("message"), Some("it <broke>"));
    }
}

#[test]
fn a_runner_error_after_two_records_keeps_them_ahead_of_it() {
    let records = [record("a", "passed", false), record("b", "failed", true)];
    let xml = stop_document(&records, StopKind::Runner, "sink busy", &suite(50));
    let doc = parse(&xml);
    let root = doc.root_element();
    assert_eq!(
        counts(root),
        ["3", "1", "1", "0", "0.000"].map(String::from)
    );
    let names: Vec<_> = testcases(&doc)
        .iter()
        .map(|c| c.attribute("name").unwrap())
        .collect();
    assert_eq!(names, ["a", "b", "(runner)"]);
}

#[test]
fn hostile_names_and_output_still_parse() {
    let mut rec = record("t\"<&>", "failed", true);
    rec.module = Some("m::\u{1b}".to_string());
    rec.message = Some("]]>\r\n\t".to_string());
    rec.stdout = "<b>&]]>\u{1b}[31m\u{0}".to_string();
    rec.stderr = "\u{ffff}".to_string();
    let xml = document(&[rec], &suite(0));
    let doc = parse(&xml);
    let case = testcases(&doc)[0];
    assert_eq!(case.attribute("name"), Some("t\"<&>"));
    assert_eq!(
        child(case, "system-out").unwrap().text(),
        Some("<b>&]]>\\u{1b}[31m\\u{0}")
    );
}

#[test]
fn the_document_starts_with_the_xml_declaration() {
    let xml = document(&[], &suite(0));
    assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites "));
    assert!(xml.ends_with("</testsuites>"));
}
