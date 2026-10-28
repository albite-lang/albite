//! The at-most-once buffer rule (ADR 6.10.26m D3, AC 3), over injected
//! Core terms and signatures. The embedded sources are checked beside the
//! core's other rules, in `native_core/tests.rs`.

use super::linear::{buffer_signatures, reused_buffers, BufferSignatures, MKBYTES_SCRUTINEE};
use super::{check_core_rules, CoreRuleViolation, DefinitionFacts};
use albite_core::terms::Term;
use albite_core::types::Type;

fn facts(name: &str) -> DefinitionFacts {
    DefinitionFacts {
        name: name.to_string(),
        ..DefinitionFacts::default()
    }
}

fn var(name: &str) -> Box<Term> {
    Box::new(Term::Var(name.to_string()))
}

/// `tg_bytes_len(name)`.
fn len_of(name: &str) -> Term {
    Term::ExternCall(
        "tg_bytes_len".to_string(),
        vec![Term::Var(name.to_string())],
    )
}

/// `λname:Nat. body` — a buffer reaches Core as a `Nat` (P0.3).
fn lambda(name: &str, body: Term) -> Term {
    Term::Lambda(name.to_string(), Type::Nat, Box::new(body))
}

fn let_in(name: &str, definition: Term, body: Term) -> Term {
    Term::Let(
        name.to_string(),
        Type::Nat,
        Box::new(definition),
        Box::new(body),
    )
}

fn signatures_with(returns: &[&str], params: &[(&str, &str)]) -> BufferSignatures {
    BufferSignatures {
        returns_buffer: returns.iter().map(ToString::to_string).collect(),
        buffer_params: params
            .iter()
            .map(|&(function, param)| (function.to_string(), vec![param.to_string()]))
            .collect(),
    }
}

// 6.10.26m AC 3: a `Bytes` parameter used twice is refused by
// `check_core_rules`, naming the definition and the variable.
#[test]
fn a_buffer_parameter_used_twice_is_refused_naming_both() {
    let twice = lambda(
        "b",
        Term::Pair(Box::new(len_of("b")), Box::new(len_of("b"))),
    );
    let signatures = signatures_with(&[], &[("twice", "b")]);
    let reused = reused_buffers("bytes::twice", &twice, &signatures);
    assert_eq!(reused, vec!["b".to_string()]);
    let facts = DefinitionFacts {
        reused_buffers: reused,
        ..facts("twice")
    };
    let violations = check_core_rules(&[facts], &[], &[]);
    assert_eq!(
        violations,
        vec![CoreRuleViolation::BufferReused {
            definition: "twice".to_string(),
            variable: "b".to_string(),
        }]
    );
    assert_eq!(
        violations[0].to_string(),
        "twice: uses the buffer `b` more than once"
    );
    // The same body over a parameter the source does not declare `Bytes`.
    assert_eq!(
        reused_buffers("twice", &twice, &BufferSignatures::default()),
        Vec::<String>::new()
    );
}

// 6.10.26m AC 3: the `MkBytes` handle is found by the pattern, not its type
// (it is a `Nat`): the handle used twice is refused, the scrutinee used once
// is not.
#[test]
fn an_mkbytes_handle_used_twice_is_refused() {
    let body = let_in(
        MKBYTES_SCRUTINEE,
        Term::Var("b".to_string()),
        let_in(
            "h",
            Term::Var(MKBYTES_SCRUTINEE.to_string()),
            Term::Pair(Box::new(len_of("h")), Box::new(len_of("h"))),
        ),
    );
    let term = lambda("b", body);
    assert_eq!(
        reused_buffers("peek", &term, &BufferSignatures::default()),
        vec!["h".to_string()]
    );
}

// 6.10.26m AC 3: match arms are SUMMED, so a buffer used once in each of two
// arms is refused although each path uses it once — D3's accepted
// conservatism.
#[test]
fn a_buffer_used_once_in_each_of_two_arms_is_refused() {
    let arms = Term::Case(
        var("c"),
        "l".to_string(),
        Box::new(len_of("b")),
        "r".to_string(),
        Box::new(len_of("b")),
    );
    let signatures = signatures_with(&[], &[("either", "b")]);
    assert_eq!(
        reused_buffers("either", &lambda("b", arms), &signatures),
        vec!["b".to_string()]
    );
}

// 6.10.26m AC 3: a `let` bound to a buffer-returning call is a buffer; the
// core's own threading shape — each use a fresh binding — is accepted, and a
// non-buffer `let` of the same name shadows it.
#[test]
fn a_threaded_buffer_is_accepted_and_a_reused_call_result_is_not() {
    let new = || {
        Term::App(
            Box::new(Term::Global("bytes::bytes_new".to_string())),
            Box::new(Term::Zero),
        )
    };
    let signatures = signatures_with(&["bytes_new"], &[]);
    let reused = let_in(
        "b",
        new(),
        Term::Pair(Box::new(len_of("b")), Box::new(len_of("b"))),
    );
    assert_eq!(
        reused_buffers("f", &reused, &signatures),
        vec!["b".to_string()]
    );
    let threaded = let_in(
        "b",
        new(),
        let_in("b", Term::Var("b".to_string()), len_of("b")),
    );
    assert_eq!(
        reused_buffers("f", &threaded, &signatures),
        Vec::<String>::new()
    );
    let shadowed = let_in(
        "b",
        new(),
        let_in("b", Term::Zero, Term::Pair(var("b"), var("b"))),
    );
    assert_eq!(
        reused_buffers("f", &shadowed, &signatures),
        Vec::<String>::new()
    );
}

// 6.10.26m AC 3: binding an unrelated name shadows only that name — a buffer
// still in scope stays one, so a copy of it bound later is a buffer too.
#[test]
fn an_unrelated_binder_keeps_other_buffers_in_scope() {
    let signatures = signatures_with(&["bytes_new"], &[]);
    let new = Term::App(
        Box::new(Term::Global("bytes::bytes_new".to_string())),
        Box::new(Term::Zero),
    );
    let term = let_in(
        "b",
        new,
        let_in(
            "n",
            Term::Zero,
            let_in(
                "c",
                Term::Var("b".to_string()),
                Term::Pair(Box::new(len_of("c")), Box::new(len_of("c"))),
            ),
        ),
    );
    assert_eq!(
        reused_buffers("f", &term, &signatures),
        vec!["c".to_string()]
    );
}

#[test]
fn signatures_read_bytes_parameters_and_returns_qualified_or_not() {
    let text = "pub fn f(b: Bytes, n: Nat) -> Bytes { b }\nfn g(x: bytes::Bytes) -> Nat { 0 }\nfn k(n: Nat) -> Nat { n }\n";
    let signatures = buffer_signatures(&[text]);
    assert_eq!(
        signatures,
        signatures_with(&["f"], &[("f", "b"), ("g", "x")])
    );
}
