//! Tests for hint helper functions: formatting, shell safety, type extraction.
//! The JSON-output tests moved to `diagnostics/json/tests.rs` (ADR 28.9.26e).

use super::*;
use crate::ElabErrorKind;
use albite_core::Type;

// ─────────────────────────────────────────────────────────────────────────────
// Format
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_format_hints_output() {
    let hints = vec![DiagnosticHint {
        command: "albite info type-encoding Nat test.tg".to_string(),
        reason: "reason".to_string(),
    }];
    let output = format_hints(&hints);
    assert!(output.contains("hint: run `albite info type-encoding Nat test.tg`"));
}

#[test]
fn test_format_hints_empty() {
    let hints: Vec<DiagnosticHint> = vec![];
    let output = format_hints(&hints);
    assert!(output.is_empty());
}

#[test]
fn test_format_hints_two_hints() {
    let hints = vec![
        DiagnosticHint {
            command: "albite info type-encoding List test.tg".to_string(),
            reason: "reason 1".to_string(),
        },
        DiagnosticHint {
            command: "albite doctor suggest-tools \"type mismatch\"".to_string(),
            reason: "reason 2".to_string(),
        },
    ];
    let output = format_hints(&hints);
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("info type-encoding"));
    assert!(lines[1].contains("suggest-tools"));
}

// ─────────────────────────────────────────────────────────────────────────────
// Shell safety
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_shell_safe_path_simple() {
    let path = std::path::Path::new("examples/list.tg");
    assert_eq!(shell_safe_path(path), "examples/list.tg");
}

#[test]
fn test_shell_safe_path_with_spaces() {
    let path = std::path::Path::new("my project/list.tg");
    let safe = shell_safe_path(path);
    assert!(safe.starts_with('\''));
    assert!(safe.ends_with('\''));
}

#[test]
fn test_shell_safe_path_with_dollar() {
    let path = std::path::Path::new("$HOME/list.tg");
    let safe = shell_safe_path(path);
    assert_eq!(safe, "'$HOME/list.tg'");
}

#[test]
fn test_shell_safe_path_with_backslash() {
    let path = std::path::Path::new("path\\to\\list.tg");
    let safe = shell_safe_path(path);
    assert!(safe.starts_with('\''));
    assert!(safe.ends_with('\''));
}

#[test]
fn test_shell_safe_path_with_single_quote() {
    let path = std::path::Path::new("it's/list.tg");
    let safe = shell_safe_path(path);
    // Single quotes in the path are escaped: 'it'\''s/list.tg'
    assert!(
        safe.contains("'\\''"),
        "Expected escaped single quote, got: {safe}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// extract_type_name
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_extract_type_name_app() {
    let ty = Type::App("Option".to_string(), vec![Type::Nat]);
    assert_eq!(extract_type_name(&ty), Some("Option".to_string()));
}

#[test]
fn test_extract_type_name_adt() {
    let ty = Type::Adt("List".to_string(), vec![Type::Nat], vec![]);
    assert_eq!(extract_type_name(&ty), Some("List".to_string()));
}

#[test]
fn test_extract_type_name_mu_recurses() {
    let inner = Type::App("Tree".to_string(), vec![]);
    let ty = Type::Mu("α".to_string(), Box::new(inner));
    assert_eq!(extract_type_name(&ty), Some("Tree".to_string()));
}

#[test]
fn test_extract_type_name_primitive_returns_none() {
    assert_eq!(extract_type_name(&Type::Nat), None);
    assert_eq!(extract_type_name(&Type::Bool), None);
    assert_eq!(extract_type_name(&Type::String), None);
}

#[test]
fn test_extract_type_name_arrow_returns_none() {
    let ty = Type::Arrow(Box::new(Type::Nat), Box::new(Type::Bool));
    assert_eq!(extract_type_name(&ty), None);
}
