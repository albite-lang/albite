//! The self-hosted error catalogue's lookup, rendering and range discipline.
//!
//! Split from `mod.rs` (ADR 19.8.26d review): the catalogue grows one entry per
//! self-hosted error code, so the file was heading for its size threshold with
//! the tests riding along. They assert what a *reader* of `explain error
//! --self-hosted` gets, which is a different question from how the lookup is
//! implemented.

use super::*;

/// The self-host's syntax kinds, the code each claims, and the bootstrap kind
/// and code it mirrors at D4's fixed offset (bootstrap E0110+k = E0800+k).
const SYNTAX_KINDS: &[(&str, &str, &str, &str)] = &[
    ("E0800", "SyntaxUnexpectedToken", "UnexpectedToken", "E0110"),
    ("E0801", "SyntaxUnexpectedEof", "UnexpectedEof", "E0111"),
    ("E0802", "SyntaxExpected", "Expected", "E0112"),
    // ADR 7.10.26f AC4: a token that cannot start a pattern.
    ("E0803", "SyntaxInvalidPattern", "InvalidPattern", "E0113"),
    ("E0806", "SyntaxReservedKeyword", "ReservedKeyword", "E0116"),
    ("E0807", "SyntaxLexical", "Lexical", "E0117"),
];

// ADR 30.9.26h AC5: every syntax code the self-host raises resolves by code
// and by name, in category "Syntax", and names its bootstrap equivalent.
#[test]
fn every_syntax_code_resolves_by_code_and_name() {
    for &(code, name, bootstrap, _) in SYNTAX_KINDS {
        assert_eq!(lookup(code).map(|e| e.name), Some(name), "by code: {code}");
        assert_eq!(
            lookup(&code.to_ascii_lowercase()).map(|e| e.name),
            Some(name),
            "case-insensitive: {code}"
        );
        assert_eq!(lookup(name).map(|e| e.code), Some(code), "by name: {name}");
        assert_eq!(lookup(code).map(|e| e.category), Some("Syntax"));
        assert_eq!(
            bootstrap_equivalent(code),
            Some(bootstrap),
            "crossref: {code}"
        );
    }
    // The reserved slots stay unanswered (29.9.26f D3: catalogued = raised).
    for reserved in ["E0804", "E0805", "E0808", "E0899"] {
        assert!(lookup(reserved).is_none(), "reserved: {reserved}");
    }
}

// ADR 30.9.26h AC5: the codes are distinct, sit in E0800-E0899, and follow
// D4's fixed offset from the bootstrap's E0110-E0117.
#[test]
fn syntax_codes_sit_in_the_syntax_block() {
    let mut seen = std::collections::HashSet::new();
    for &(code, _, _, bootstrap_code) in SYNTAX_KINDS {
        let number: u32 = code[1..].parse().expect("Ennnn");
        let bootstrap_number: u32 = bootstrap_code[1..].parse().expect("Ennnn");
        assert!((800..=899).contains(&number), "outside the block: {code}");
        assert_eq!(number - 800, bootstrap_number - 110, "offset: {code}");
        assert!(seen.insert(code), "claimed twice: {code}");
    }
    let catalogued = SELF_HOSTED_ERRORS
        .iter()
        .filter(|e| e.category == "Syntax")
        .count();
    assert_eq!(
        catalogued,
        SYNTAX_KINDS.len(),
        "every Syntax entry is listed"
    );
    assert!(render_error_list_body().contains("Syntax:\n"));
}

#[test]
fn lookup_by_code() {
    let entry = SELF_HOSTED_ERRORS.iter().find(|e| e.code == "E0001");
    assert!(entry.is_some());
    assert_eq!(entry.unwrap().name, "ErrTypeMismatch");
}

#[test]
fn lookup_by_name() {
    let entry = SELF_HOSTED_ERRORS
        .iter()
        .find(|e| e.name.eq_ignore_ascii_case("ErrUnresolvedValue"));
    assert!(entry.is_some());
    assert_eq!(entry.unwrap().code, "E0101");
}

#[test]
fn the_listing_carries_its_header_body_and_footer() {
    let listing = self_hosted_error_list();
    assert!(listing.starts_with("Self-Hosted Compiler Error Reference\n"));
    assert!(listing.contains("See `albite explain error` for bootstrap codes."));
    assert!(listing.contains(&render_error_list_body()), "body missing");
    assert!(listing.ends_with("to look up by name.\n"), "footer missing");
}

#[test]
fn listing_body_groups_every_entry_under_its_own_category() {
    let body = render_error_list_body();
    assert!(!body.is_empty(), "the listing body must not be empty");
    // Every catalogue entry appears exactly once, under a heading — a
    // dropped category or a mis-filtered group would lose entries silently.
    for entry in SELF_HOSTED_ERRORS.iter() {
        assert_eq!(
            body.matches(entry.code).count(),
            1,
            "{} listed {} time(s)",
            entry.code,
            body.matches(entry.code).count()
        );
        assert!(body.contains(entry.name), "{} missing", entry.name);
    }
    // A heading is present exactly for the categories that have entries.
    for cat in CATEGORY_ORDER {
        let used = SELF_HOSTED_ERRORS.iter().any(|e| e.category == *cat);
        assert_eq!(
            body.contains(&format!("{cat}:\n")),
            used,
            "heading `{cat}` present={}, used={used}",
            body.contains(&format!("{cat}:\n"))
        );
    }
}

#[test]
fn short_desc_keeps_the_first_sentence_only() {
    // The listing shows one line per code, so a multi-sentence description
    // is cut at the first period — the period and everything after it go.
    assert_eq!(short_desc("First. Second. Third."), "First");
    // No period: the whole string survives unchanged.
    assert_eq!(short_desc("no trailing period"), "no trailing period");
    assert_eq!(short_desc(""), "");
    // A leading period yields an empty summary rather than panicking.
    assert_eq!(short_desc(".leading"), "");
}

#[test]
fn listing_rows_show_each_entrys_first_sentence() {
    // Ties the rendered listing to short_desc: every row must carry the
    // real summary, not a placeholder or an empty string.
    let body = render_error_list_body();
    for entry in SELF_HOSTED_ERRORS.iter() {
        let summary = short_desc(entry.description);
        assert!(!summary.is_empty(), "{} has an empty summary", entry.code);
        assert!(
            body.contains(summary),
            "{} row is missing its summary {summary:?}",
            entry.code
        );
    }
}

#[test]
fn listing_body_follows_the_declared_category_order() {
    let body = render_error_list_body();
    let mut last = 0usize;
    for cat in CATEGORY_ORDER {
        if let Some(at) = body.find(&format!("{cat}:\n")) {
            assert!(at >= last, "category `{cat}` is out of declared order");
            last = at;
        }
    }
}

#[test]
fn bootstrap_crossref_exists() {
    assert_eq!(bootstrap_equivalent("E0001"), Some("TypeMismatch"));
    assert_eq!(bootstrap_equivalent("E0101"), Some("UndefinedVariable"));
    assert_eq!(bootstrap_equivalent("E0999"), None);
}

/// ADR 4.10.26l D4: the self-host's glob conflict is catalogued by code and
/// by name, and cross-references the bootstrap's `GlobConflict` (E0018).
#[test]
fn the_glob_conflict_code_resolves_and_names_its_bootstrap_kind() {
    assert_eq!(lookup("E0108").map(|e| e.name), Some("ErrGlobConflict"));
    assert_eq!(lookup("ErrGlobConflict").map(|e| e.code), Some("E0108"));
    assert_eq!(bootstrap_equivalent("E0108"), Some("GlobConflict"));
}

/// ADR 7.10.26e AC3: the self-host's ambiguous bare value is the bootstrap's
/// `AmbiguousValue` (E0019), and its description states the fall-through rule.
#[test]
fn the_ambiguous_bare_value_code_names_its_bootstrap_kind() {
    assert_eq!(bootstrap_equivalent("E0105"), Some("AmbiguousValue"));
    let entry = lookup("E0105").expect("E0105 catalogued");
    assert_eq!(entry.name, "ErrAmbiguousName");
    assert!(entry.description.contains("neither defines nor imports"));
    assert!(!entry.description.contains("glob"));
}

/// ADR 7.10.26f AC 4: a token that cannot start a pattern is the bootstrap's
/// `InvalidPattern` (E0113), at the fixed offset's own number.
#[test]
fn the_invalid_pattern_code_names_its_bootstrap_kind() {
    assert_eq!(bootstrap_equivalent("E0803"), Some("InvalidPattern"));
    let entry = lookup("E0803").expect("E0803 catalogued");
    assert_eq!(entry.name, "SyntaxInvalidPattern");
    assert_eq!(entry.category, "Syntax");
    assert!(entry.description.contains("cannot start a pattern"));
}

/// ADR 6.10.26k AC4 (D7): the self-host's two type-collection refusals
/// resolve, and name the bootstrap kinds the records gate pairs them with.
#[test]
fn the_type_collection_refusals_name_their_bootstrap_kinds() {
    assert_eq!(
        lookup("E0107").map(|e| e.name),
        Some("ErrBuiltinTypeRedefined")
    );
    assert_eq!(bootstrap_equivalent("E0107"), Some("BuiltinTypeRedefined"));
    assert_eq!(lookup("E0204").map(|e| e.name), Some("ErrCyclicDependency"));
    assert_eq!(bootstrap_equivalent("E0204"), Some("RecursiveAlias"));
}

/// ADR 6.10.26j AC8: the five package codes resolve by code and by name, in
/// the Packages category, and name the bootstrap kind of the same number.
#[test]
fn the_package_codes_resolve_and_name_their_bootstrap_kinds() {
    let expected = [
        ("E0120", "ErrManifestInvalid", "ManifestInvalid"),
        ("E0121", "ErrDependencyNotFound", "DependencyNotFound"),
        ("E0122", "ErrPackageNameConflict", "PackageNameConflict"),
        (
            "E0123",
            "ErrPackageExternCollision",
            "CrossPackageNameCollision",
        ),
        ("E0124", "ErrDependencyCycle", "DependencyCycle"),
    ];
    for (code, name, bootstrap) in expected {
        let entry = lookup(code).unwrap_or_else(|| panic!("{code} uncatalogued"));
        assert_eq!((entry.name, entry.category), (name, "Packages"));
        assert_eq!(lookup(name).map(|e| e.code), Some(code));
        assert_eq!(bootstrap_equivalent(code), Some(bootstrap));
    }
    assert!(render_error_list_body().contains("Packages:\n"));
}

#[test]
fn all_entries_have_unique_codes() {
    let mut codes: Vec<&str> = SELF_HOSTED_ERRORS.iter().map(|e| e.code).collect();
    codes.sort();
    codes.dedup();
    assert_eq!(codes.len(), SELF_HOSTED_ERRORS.len());
}

/// AC9. `explain error --self-hosted` resolves the mirror's code, by code
/// and by kind name — the failure this catches is a diagnostic the
/// compiler emits and its own explainer calls unknown.
#[test]
fn the_positivity_code_resolves_by_code_and_by_name() {
    assert_eq!(
        lookup("E0700").map(|e| e.name),
        Some("ErrNonStrictlyPositive")
    );
    assert_eq!(
        lookup("e0700").map(|e| e.name),
        Some("ErrNonStrictlyPositive")
    );
    assert_eq!(
        lookup("ErrNonStrictlyPositive").map(|e| e.code),
        Some("E0700")
    );
    assert_eq!(
        lookup("errnonstrictlypositive").map(|e| e.code),
        Some("E0700")
    );
}

/// The termination mirror claimed the two codes E0710 reserved for it
/// (ADR 19.8.26d D3), and they resolve **separately**: a proof reaching a
/// partial constant and a recursion that does not descend are different
/// repairs, so a lookup that answered both with one entry would be wrong
/// in the half a reader is more likely to arrive with.
#[test]
fn both_termination_codes_resolve_to_their_own_entry() {
    assert_eq!(
        lookup("E0710").map(|e| e.name),
        Some("ErrCannotProveTermination")
    );
    assert_eq!(lookup("E0711").map(|e| e.name), Some("ErrPartialInProof"));
    assert_eq!(
        lookup("ErrPartialInProof").map(|e| e.code),
        Some("E0711"),
        "by name as well as by code"
    );
}

/// The rest of E0700-E0799 is RESERVED, not catalogued: a reserved code
/// resolves to nothing rather than to its neighbour, so a later gate
/// claiming one finds it free rather than already answered.
#[test]
fn a_reserved_soundness_code_resolves_to_nothing() {
    assert_eq!(lookup("E0705").map(|e| e.code), None);
    assert_eq!(lookup("E0712").map(|e| e.code), None);
}

#[test]
fn fuzzy_finds_partial_code() {
    assert_eq!(fuzzy_match_self_hosted("E000"), Some("E0001"));
}

#[test]
fn fuzzy_finds_partial_name() {
    assert_eq!(fuzzy_match_self_hosted("mismatch"), Some("ErrTypeMismatch"));
}

// 2.10.26i AC3: a bootstrap number spelled at a coded call or in a code arm is
// reported; a catalogued code, a passed-through code and the cascade marker are
// not, and neither is a call quoted in a comment.
#[test]
fn uncatalogued_code_literals_are_reported() {
    let source = r#"
        elab_expr_error_coded("type mismatch", span, "E0010")
        elab_expr_error_coded("unknown value: x", span, "E0101")
        elab_type_error_coded(e.message, span, e.code)
        ErrCoded("E0011", "msg")
        ErrFoo(_) => "E0012",
        ErrBar(_) => "E0001",
        elab_expr_error_coded("m (see \"x\")", span, "cascade-after-invalid-type")
        // elab_expr_error_coded("quoted", span, "E0013")
        let s = "elab_expr_error_coded(\"E0014\")";
    "#;
    assert_eq!(
        super::literals::uncatalogued_code_literals(source),
        vec!["E0010", "E0011", "E0012"]
    );
}

// 2.10.26i AC3: a call split across lines is read to its closing paren.
#[test]
fn a_split_call_literal_is_still_read() {
    let source = "elab_expr_error_coded(\n    msg,\n    f(span),\n    \"E0010\")\n";
    assert_eq!(
        super::literals::uncatalogued_code_literals(source),
        vec!["E0010"]
    );
}

// 2.10.26i AC3 (D9): every code the self-host's elaborator spells is catalogued.
#[test]
fn every_selfhost_elaborator_code_is_catalogued() {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("readable elaborator dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "tg") {
                out.push(path);
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/compiler/elab");
    let mut files = Vec::new();
    walk(&root, &mut files);
    assert!(
        files.len() > 100,
        "scanned only {} files under {}",
        files.len(),
        root.display()
    );
    let found: Vec<String> = files
        .iter()
        .flat_map(|f| {
            let source = std::fs::read_to_string(f).expect("readable .tg");
            super::literals::uncatalogued_code_literals(&source)
                .into_iter()
                .map(move |code| format!("{}: {code}", f.display()))
        })
        .collect();
    assert!(
        found.is_empty(),
        "uncatalogued self-host codes:\n{}",
        found.join("\n")
    );
}
