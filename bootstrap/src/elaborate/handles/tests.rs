//! Tests for the affine handle rule (ADR 7.10.26t AC 1, AC 2): the list, the
//! kinds and the "contains" walk over parsed declarations, then the use rule
//! over parsed function bodies. Pure functions over source strings — no
//! elaboration, no LLVM.

use super::*;
use crate::ast::{Item, TypeExpr};

/// The two handle types and their API, as the compiler's builder module and
/// the native core declare them (bodies elided to the wrapper).
const PRELUDE: &str = r#"
pub type StringBuilder = MkStringBuilder(Nat)
pub type Bytes = MkBytes(Nat)
fn string_builder_new() -> StringBuilder { MkStringBuilder(0) }
fn push_str(sb: StringBuilder, s: String) -> StringBuilder { sb }
fn string_builder_len(sb: StringBuilder) -> Nat { 0 }
fn to_string(sb: StringBuilder) -> String { "" }
fn bytes_new(n: Nat) -> Bytes { MkBytes(n) }
fn bytes_len(b: Bytes) -> Nat { 0 }
fn bytes_get(b: Bytes, i: Nat) -> Nat { 0 }
fn bytes_set(b: Bytes, i: Nat, v: Nat) -> Bytes { b }
fn bytes_into_string(b: Bytes) -> String { "" }
"#;

fn items_of(source: &str) -> Vec<Item> {
    let (parsed, errors) = crate::parser::parse(source);
    assert!(errors.is_empty(), "fixture must parse: {errors:?}");
    parsed.items
}

fn declarations(source: &str) -> HandleDeclarations {
    HandleDeclarations::from_items(&items_of(&format!("{PRELUDE}{source}")))
}

/// The declared result type of the fixture function `probe`.
fn probe_result(source: &str) -> (HandleDeclarations, TypeExpr) {
    let items = items_of(&format!("{PRELUDE}{source}"));
    let result = items
        .iter()
        .find_map(|item| match item {
            Item::Function(f) if f.name.name == "probe" => f.return_type.clone(),
            _ => None,
        })
        .expect("fixture declares `probe` with a result type");
    (HandleDeclarations::from_items(&items), result)
}

fn contains(source: &str) -> bool {
    let (declarations, result) = probe_result(source);
    declarations.contains_handle(&result)
}

// 7.10.26t AC1: "contains a handle" over declared types.
#[test]
fn a_handle_is_contained_by_name_through_fields_and_through_arguments() {
    assert!(contains("fn probe() -> StringBuilder { probe() }"));
    assert!(contains("fn probe() -> Bytes { probe() }"));
    assert!(contains(
        "type W = MkW(StringBuilder)\nfn probe() -> W { probe() }"
    ));
    assert!(contains(
        "type Opt<T> = No | So(T)\nfn probe() -> Opt<Bytes> { probe() }"
    ));
    // A recursive type with a `Bytes` field: the walk ends on its visited set.
    assert!(contains(
        "type Chain = End | Link(Bytes, Chain)\nfn probe() -> Chain { probe() }"
    ));
    assert!(contains(
        "type Loop = Step(Loop) | Hold(Bytes)\nfn probe() -> Loop { probe() }"
    ));
}

// 7.10.26t AC1: and is false where nothing reaches a listed type.
#[test]
fn nat_a_nat_wrapper_and_a_type_parameter_contain_no_handle() {
    assert!(!contains("fn probe() -> Nat { 0 }"));
    assert!(!contains(
        "type Box = MkBox(Nat)\nfn probe() -> Box { probe() }"
    ));
    assert!(!contains("fn probe<T>(x: T) -> T { x }"));
    assert!(!contains(
        "type Spin = Spin(Spin)\nfn probe() -> Spin { probe() }"
    ));
    // A closure that returns a handle is not one.
    assert!(!contains("fn probe() -> Nat -> Bytes { probe() }"));
}

// 7.10.26t AC1: every D3 name has its kind when its signature mentions a
// handle, and none over `String`.
#[test]
fn each_table_name_has_its_kind_only_over_a_handle_signature() {
    let decls = declarations("");
    assert_eq!(
        decls.kind_of("string_builder_new"),
        Some(HandleFunctionKind::Maker)
    );
    assert_eq!(decls.kind_of("bytes_new"), Some(HandleFunctionKind::Maker));
    assert_eq!(
        decls.kind_of("string_builder_len"),
        Some(HandleFunctionKind::Observer)
    );
    assert_eq!(
        decls.kind_of("bytes_len"),
        Some(HandleFunctionKind::Observer)
    );
    assert_eq!(
        decls.kind_of("bytes_get"),
        Some(HandleFunctionKind::Observer)
    );
    assert_eq!(
        decls.kind_of("push_str"),
        Some(HandleFunctionKind::Threader)
    );
    assert_eq!(
        decls.kind_of("bytes_set"),
        Some(HandleFunctionKind::Threader)
    );
    assert_eq!(
        decls.kind_of("to_string"),
        Some(HandleFunctionKind::Consumer)
    );
    assert_eq!(
        decls.kind_of("bytes_into_string"),
        Some(HandleFunctionKind::Consumer)
    );
    assert_eq!(decls.kind_of("unlisted"), None);
    let over_string = HandleDeclarations::from_items(&items_of(
        "fn to_string(s: String) -> String { s }\nfn bytes_len(s: String) -> Nat { 0 }",
    ));
    assert_eq!(over_string.kind_of("to_string"), None);
    assert_eq!(over_string.kind_of("bytes_len"), None);
    assert!(decls.returns_handle("push_str"));
    assert!(!decls.returns_handle("to_string"));
    assert!(!decls.returns_handle("unlisted"));
}

// 7.10.26t AC1: the table covers every D3 name, once.
#[test]
fn the_kind_table_lists_fourteen_names_once_each() {
    let mut names: Vec<&str> = FUNCTION_KINDS.iter().map(|(name, _)| *name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), 14);
    assert!(is_handle_type_name("StringBuilder") && is_handle_type_name("Bytes"));
    assert!(!is_handle_type_name("Box"));
}

/// Every finding in the fixture's function `f`, as (kind, handle, text at the span).
fn findings_with(source: &str, policy: UsePolicy) -> Vec<(HandleFindingKind, String, String)> {
    let full = format!("{PRELUDE}{source}");
    let items = items_of(&full);
    let declarations = HandleDeclarations::from_items(&items);
    let function = items
        .iter()
        .find_map(|item| match item {
            Item::Function(f) if f.name.name == "f" => Some(f),
            _ => None,
        })
        .expect("fixture declares `f`");
    check_function(function, &declarations, policy)
        .into_iter()
        .map(|finding| {
            let text = full[finding.span.start as usize..finding.span.end as usize].to_string();
            (finding.kind, finding.handle, text)
        })
        .collect()
}

pub(super) fn findings(source: &str) -> Vec<(HandleFindingKind, String, String)> {
    findings_with(source, UsePolicy::AFFINE)
}

fn used_after(handle: &str) -> (HandleFindingKind, String, String) {
    (
        HandleFindingKind::UsedAfterConsumed,
        handle.into(),
        handle.into(),
    )
}

// 7.10.26t AC2: the refusals, each naming its handle at the offending use.
#[test]
fn a_second_consuming_use_and_an_observer_after_one_are_refused() {
    let twice = "fn f(sb: StringBuilder) -> String {
        let a = push_str(sb, \"x\");
        let b = push_str(sb, \"y\");
        to_string(a) ++ to_string(b)
    }";
    assert_eq!(findings(twice), vec![used_after("sb")]);
    let observed = "fn f(sb: StringBuilder) -> Nat {
        let s = to_string(sb);
        string_builder_len(sb)
    }";
    assert_eq!(findings(observed), vec![used_after("sb")]);
    let moved = "fn f(a: StringBuilder) -> String {
        let b = a;
        to_string(a)
    }";
    assert_eq!(findings(moved), vec![used_after("a")]);
}

// 7.10.26t AC2: a closure that mentions an outer handle.
#[test]
fn a_closure_mentioning_an_outer_handle_is_refused() {
    let captured = "fn f(sb: StringBuilder) -> String {
        let g = |s: String| to_string(push_str(sb, s));
        g(\"x\")
    }";
    assert_eq!(
        findings(captured),
        vec![(
            HandleFindingKind::CapturedByClosure,
            "sb".into(),
            "sb".into()
        )]
    );
    // A closure's own parameter of the same name shadows the outer handle.
    let shadowed = "fn f(sb: StringBuilder) -> String {
        let g = |sb: Nat| sb;
        to_string(sb)
    }";
    assert!(findings(shadowed).is_empty());
}

// 7.10.26t AC2: an `if` yielding a handle makes its `let` a handle variable.
#[test]
fn a_let_of_an_if_yielding_a_handle_is_tracked() {
    let source = "fn f(sb: StringBuilder, c: Bool) -> String {
        let sb2 = if c { push_str(sb, \"a\") } else { sb };
        let s = to_string(sb2);
        to_string(sb2)
    }";
    assert_eq!(findings(source), vec![used_after("sb2")]);
}

// 7.10.26t AC2: the accepted shapes — one consuming use on every path.
#[test]
fn shadowing_observers_arms_and_threading_are_accepted() {
    let accepted = [
        "fn f(sb: StringBuilder) -> String {
            let sb = push_str(sb, \"a\");
            let sb = push_str(sb, \"b\");
            to_string(sb)
        }",
        "fn f(sb: StringBuilder) -> String {
            let n = string_builder_len(sb);
            to_string(sb)
        }",
        "fn f(sb: StringBuilder, c: Bool) -> String {
            if c { to_string(sb) } else { to_string(sb) }
        }",
        "type Two = One | Other
        fn f(sb: StringBuilder, t: Two) -> String {
            match t { One => to_string(sb), Other => to_string(push_str(sb, \"x\")) }
        }",
        "fn f(sb: StringBuilder, c: Bool) -> StringBuilder {
            if c { sb } else { push_str(sb, \"x\") }
        }",
        "fn f(a: StringBuilder, b: Bytes) -> String {
            to_string(a) ++ bytes_into_string(b)
        }",
        "fn wrap(sb: StringBuilder) -> StringBuilder { push_str(sb, \"w\") }
        fn f(sb: StringBuilder) -> String {
            let sb = wrap(sb);
            to_string(wrap(sb))
        }",
        "fn f(sb: StringBuilder, c: Bool) -> String {
            if c { return to_string(sb); };
            to_string(sb)
        }",
    ];
    for source in accepted {
        assert!(findings(source).is_empty(), "accepted: {source}");
    }
}

// 7.10.26t AC2: tracking stops at the pattern — `BufferReused`'s scrutinee
// key is not `B4.1`'s, so the raw `Nat` may be used twice.
#[test]
fn the_raw_nat_a_handle_pattern_binds_is_not_tracked() {
    let source = "fn f(sb: StringBuilder) -> Nat {
        match sb { MkStringBuilder(h) => h + h }
    }";
    assert!(findings(source).is_empty());
}

// 7.10.26t AC2: the must-fail twin is the policy. The accepted arm case
// under `BufferReused`'s summed arms is refused, so `Max` is live.
#[test]
fn the_accepted_arm_case_is_refused_when_arms_sum() {
    let arms = "fn f(sb: StringBuilder, c: Bool) -> String {
        if c { to_string(sb) } else { to_string(sb) }
    }";
    let summed = UsePolicy {
        observers_free: true,
        arms: ArmPolicy::Sum,
    };
    assert_eq!(findings_with(arms, summed), vec![used_after("sb")]);
    let observer = "fn f(sb: StringBuilder) -> String {
        let n = string_builder_len(sb);
        to_string(sb)
    }";
    let counted = UsePolicy {
        observers_free: false,
        arms: ArmPolicy::Max,
    };
    assert_eq!(findings_with(observer, counted), vec![used_after("sb")]);
}

// 7.10.26t AC2: each refusal is reported as its diagnostic, naming the
// handle and the definition, at the use.
#[test]
fn a_finding_becomes_its_coded_diagnostic() {
    let finding = HandleFinding {
        kind: HandleFindingKind::CapturedByClosure,
        handle: "sb".into(),
        span: crate::span::Span::new(3, 5),
    };
    let error = finding_error("deferred", finding);
    assert_eq!(error.kind.code(), "E0093");
    assert_eq!(error.span, crate::span::Span::new(3, 5));
    assert_eq!(
        error.message,
        "handle `sb` is captured by a closure, in `deferred`"
    );
    let ok = with_handle_errors(Ok(()), Vec::new());
    assert!(ok.is_ok());
    let merged = with_handle_errors(Err(vec![error.clone()]), vec![error]);
    assert_eq!(merged.map_err(|errors| errors.len()), Err(2));
}

// 7.10.26t AC7: the native core's `BufferReused` keeps its own walk and still
// sums arms — a buffer used once in each arm of a `Case` is refused there —
// while `B4.1`'s affine walk accepts the same shape in source. The two rules
// differ on purpose (D5's outcome), and this build changed neither's verdict.
#[test]
fn the_core_rule_still_sums_the_arms_b4_1_takes_the_maximum_of() {
    use crate::driver::native_core::rules::linear::{reused_buffers, BufferSignatures};
    use albite_core::terms::Term;
    use albite_core::types::Type;

    let len_of_b = || Term::ExternCall("tg_bytes_len".into(), vec![Term::Var("b".into())]);
    let arms = Term::Case(
        Box::new(Term::Var("c".into())),
        "l".into(),
        Box::new(len_of_b()),
        "r".into(),
        Box::new(len_of_b()),
    );
    let either = Term::Lambda("b".into(), Type::Nat, Box::new(arms));
    let signatures = BufferSignatures {
        returns_buffer: Default::default(),
        buffer_params: [("either".to_string(), vec!["b".to_string()])].into(),
    };
    assert_eq!(
        reused_buffers("either", &either, &signatures),
        vec!["b".to_string()]
    );
    let in_source = "fn f(b: Bytes, c: Bool) -> Nat {
        if c { bytes_len(b) + 0 } else { bytes_len(b) }
    }";
    let consumed_in_each_arm = "fn f(b: Bytes, c: Bool) -> String {
        if c { bytes_into_string(b) } else { bytes_into_string(b) }
    }";
    assert!(findings(in_source).is_empty());
    assert!(findings(consumed_in_each_arm).is_empty());
}
