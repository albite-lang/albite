//! A variable catch-all binds the whole matched value (ADR 9.10.26e AC 2,
//! AC 4): the outermost scrutinee at its declared type, under a fresh
//! `__catch_all_<Ctor>` case binder — never the payload of the constructor
//! it stands in for.

use crate::driver::native_core::rules::linear::{buffer_signatures, reused_buffers};
use crate::elaborate::tests::{elab_err, elab_ok};

/// Three constructors with three different payload types, and a function
/// that takes the whole type — `g(other)` type-checks only if `other: P`.
const THREE: &str = r#"
    type P = A(Nat) | B(Bool) | C(String)
    fn g(q: P) -> Nat { 7 }
"#;

use super::core_of;

/// How many times `needle` occurs in `haystack`.
fn occurrences(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

// 9.10.26e AC2: §1.1 row 1, a middle-slot catch-all over three constructors.
#[test]
fn a_catch_all_after_a_middle_constructor_is_the_scrutinee() {
    let defs = elab_ok(&format!(
        "{THREE} fn f(q: P) -> Nat {{ match q {{ B(x) => 1, other => g(other) }} }}"
    ));
    let core = core_of(&defs, "f");
    assert!(core.contains("__catch_all_A =>"), "{core}");
    assert!(core.contains("__catch_all_C =>"), "{core}");
    assert_eq!(occurrences(&core, "(let other : P"), 2, "{core}");
    assert_eq!(occurrences(&core, "= q in (global:g other)"), 2, "{core}");
}

// 9.10.26e AC2: §1.1 row 2, the constructor arm first.
#[test]
fn a_catch_all_after_the_first_constructor_is_the_scrutinee() {
    let defs = elab_ok(&format!(
        "{THREE} fn f(q: P) -> Nat {{ match q {{ A(x) => 1, other => g(other) }} }}"
    ));
    let core = core_of(&defs, "f");
    assert!(
        !core.contains("other =>"),
        "the name is no case binder: {core}"
    );
    assert_eq!(occurrences(&core, "= q in (global:g other)"), 2, "{core}");
}

// 9.10.26e AC2: §1.1 row 3, the 7.10.26t shape — a projection scrutinee is
// let-bound once, and the catch-all names that binding.
#[test]
fn a_projection_scrutinee_is_bound_once_and_named_by_the_catch_all() {
    let defs = elab_ok(
        r#"
        type Span = Sp(Nat)
        type Pat = PatVar(Nat) | PatWild(Span) | PatLit(String)
        type Node = { pattern: Pat, n: Nat }
        fn w(p: Pat) -> Nat { 3 }
        fn f(p: Node) -> Nat { match p.pattern { PatVar(id) => id, other => w(other) } }
    "#,
    );
    let core = core_of(&defs, "f");
    assert_eq!(occurrences(&core, "(let __scrut_"), 1, "{core}");
    assert_eq!(occurrences(&core, "(fst p)"), 1, "evaluated once: {core}");
    assert!(core.contains("adt_match __scrut_"), "{core}");
    assert_eq!(occurrences(&core, "(let other : Pat"), 2, "{core}");
}

// 9.10.26e AC3: a variable scrutinee, or no variable catch-all, binds nothing new.
#[test]
fn no_scrutinee_binding_without_a_variable_catch_all_over_a_projection() {
    let defs = elab_ok(&format!(
        "{THREE}
        type Node = {{ pattern: P, n: Nat }}
        fn wild(p: Node) -> Nat {{ match p.pattern {{ A(x) => x, _ => 0 }} }}
        fn named(q: P) -> Nat {{ match q {{ A(x) => x, other => g(other) }} }}"
    ));
    let wild = core_of(&defs, "wild");
    assert!(!wild.contains("__scrut_"), "{wild}");
    assert!(wild.contains("__catch_all_B =>"), "{wild}");
    let named = core_of(&defs, "named");
    assert!(!named.contains("__scrut_"), "{named}");
}

// 9.10.26e AC2: §1.1 row 4 — the payload reading is refused.
#[test]
fn a_catch_all_is_not_a_payload() {
    let errors = elab_err(
        r#"
        type P = A(Nat) | B(Nat)
        fn h(q: P) -> Nat { match q { B(x) => 100, other => other } }
    "#,
    );
    assert!(
        errors.iter().any(|e| e.kind.code() == "E0010"),
        "expected E0010, got: {errors:?}"
    );
}

// 9.10.26e AC2: the two-constructor Core shape.
#[test]
fn a_two_constructor_catch_all_is_let_bound_under_a_fresh_binder() {
    let defs = elab_ok(
        r#"
        type P = A(Nat) | B(Nat)
        fn g(q: P) -> Nat { 7 }
        fn h(q: P) -> Nat { match q { B(x) => 100, other => g(other) } }
    "#,
    );
    let core = core_of(&defs, "h");
    assert!(
        core.contains("inl __catch_all_A => (let other : (Nat + Nat) = q in"),
        "{core}"
    );
}

// 9.10.26e AC2: §1.1 row 5 — in a nested match, every `let other` binds the
// OUTER value, the one inside `Som` included, while an inner binder still
// binds the inner payload.
#[test]
fn a_nested_catch_all_binds_the_outer_value() {
    let defs = elab_ok(
        r#"
        type E = A | B | C
        type Opt = Non | Som(E)
        fn g(o: Opt) -> Nat { 7 }
        fn f(o: Opt) -> Nat { match o { Som(A()) => 1, Som(B()) => 2, other => g(other) } }
    "#,
    );
    let core = core_of(&defs, "f");
    assert!(core.contains("adt_match __nested_Som"), "{core}");
    assert_eq!(occurrences(&core, "(let other : (Unit + E"), 2, "{core}");
    assert_eq!(occurrences(&core, "= o in (global:g other)"), 2, "{core}");
}

// 9.10.26e AC2: an inner match's own binder names the inner value, not the
// outer one — the override follows the outer catch-all arm only.
#[test]
fn an_inner_binder_names_the_inner_value() {
    let defs = elab_ok(
        r#"
        type E = A | B | C
        type Opt = Non | Som(E)
        fn k(e: E) -> Nat { 5 }
        fn f(o: Opt) -> Nat { match o { Som(A()) => 1, Som(e) => k(e), other => 9 } }
    "#,
    );
    let core = core_of(&defs, "f");
    assert!(core.contains("= __nested_Som in (global:k e)"), "{core}");
}

// 9.10.26e AC2 / P0.6: a catch-all over a buffer scrutinee adds no reuse.
#[test]
fn a_catch_all_over_a_buffer_is_not_reused() {
    let source = r#"
        type Bytes = MkBytes(Nat)
        fn size(b: Bytes) -> Nat { match b { MkBytes(h) => h, other => 0 } }
    "#;
    let defs = elab_ok(source);
    let def = defs.iter().find(|d| d.name == "size").expect("size");
    let signatures = buffer_signatures(&[source]);
    assert_eq!(signatures.buffer_params["size"], vec!["b".to_string()]);
    assert!(reused_buffers("size", &def.term.term, &signatures).is_empty());
}

/// The tier-5 suite, whose `strip_none` is §1.2's `h`.
fn suite_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../src/compiler/test_catch_all_binder.tg")
}

// 9.10.26e AC4: the true theorem — the catch-all is the identity — is
// admitted by the module-tree walk, whose admission gate is the kernel. The
// single-file `elab` harness does not unfold globals in `refl`, so the walk is
// the only route on which this theorem says anything.
#[test]
fn the_true_theorem_is_admitted() {
    let project = crate::driver::elaborate_project_cold(&suite_path(), 20)
        .unwrap_or_else(|error| panic!("the suite is refused: {error:?}"));
    assert!(project
        .defs
        .iter()
        .any(|def| def.name.ends_with("catch_all_keeps_depth")));
}

// 9.10.26e AC4: the false one — `strip_none` strips a layer — is refused.
#[test]
fn the_false_theorem_is_refused() {
    let suite = std::fs::read_to_string(suite_path()).expect("the suite reads");
    let true_claim = "(Base(1))))), 2>";
    assert_eq!(
        suite.matches(true_claim).count(),
        1,
        "the claim is where it was"
    );
    let dir = tempfile::tempdir().expect("a scratch dir");
    let strips = dir.path().join("strips.tg");
    std::fs::write(&strips, suite.replace(true_claim, "(Base(1))))), 1>")).expect("written");
    let error = crate::driver::elaborate_project_cold(&strips, 20)
        .err()
        .expect("depth 1 is no theorem");
    assert!(format!("{error:?}").contains("refl"), "{error:?}");
}
