//! ADR 7.10.26t AC 3: each affine handle refusal beside its accepted twin,
//! through the per-module pipeline — the program-level check `expect_error`
//! cannot see, since it runs over a whole module tree after the walk, not over
//! one expression. Also a warm run: a module the elaboration cache serves still
//! has its parsed body checked (D6).
//!
//! A folder of its own because `per_module/tests/` holds its eight-file cap.

use std::sync::Mutex;

use super::conversion_discharge::{file_of, run_fresh, run_tree};
use crate::cache::BuildCache;
use crate::elaborate::ElabError;

/// The builder's API with its declared signatures.
const BUILDER: &str = "pub type StringBuilder = MkStringBuilder(Nat)\n\
    pub fn string_builder_new() -> StringBuilder { MkStringBuilder(0) }\n\
    pub fn push_str(sb: StringBuilder, s: String) -> StringBuilder { sb }\n\
    pub fn string_builder_len(sb: StringBuilder) -> Nat { 0 }\n\
    pub fn to_string(sb: StringBuilder) -> String { \"\" }\n";

/// `(code, message)` of every error, in order.
fn coded(errors: &[ElabError]) -> Vec<(&'static str, String)> {
    errors
        .iter()
        .map(|error| (error.kind.code(), error.message.clone()))
        .collect()
}

fn refused(body: &str) -> Vec<(&'static str, String)> {
    let source = format!("{BUILDER}{body}\nfn main() -> Nat {{ 0 }}\n");
    let errors = run_fresh(&[("main.tg", &source)], 1)
        .err()
        .expect("the handle rule refuses the program");
    coded(&errors)
}

fn accepted(body: &str) {
    let source = format!("{BUILDER}{body}\nfn main() -> Nat {{ 0 }}\n");
    if let Err(errors) = run_fresh(&[("main.tg", &source)], 1) {
        panic!("the twin is accepted, got {:?}", coded(&errors));
    }
}

fn used_after(handle: &str, definition: &str) -> Vec<(&'static str, String)> {
    vec![(
        "E0092",
        format!("handle `{handle}` is used after it was consumed, in `{definition}`"),
    )]
}

// 7.10.26t AC3: a second threader, and its twin that shadows the successor.
#[test]
fn two_threaders_are_refused_and_threading_the_successor_is_not() {
    let twice = "fn f(sb: StringBuilder) -> String {
        let a = push_str(sb, \"x\");
        let b = push_str(sb, \"y\");
        to_string(a) ++ to_string(b)
    }";
    assert_eq!(refused(twice), used_after("sb", "f"));
    accepted(
        "fn f(sb: StringBuilder) -> String {
            let sb = push_str(sb, \"x\");
            let sb = push_str(sb, \"y\");
            to_string(sb)
        }",
    );
}

// 7.10.26t AC3: an observer after the consumer, and the observer before it.
#[test]
fn an_observer_after_the_consumer_is_refused_and_before_it_is_not() {
    let after = "fn f(sb: StringBuilder) -> Nat {
        let s = to_string(sb);
        string_builder_len(sb)
    }";
    assert_eq!(refused(after), used_after("sb", "f"));
    accepted(
        "fn f(sb: StringBuilder) -> String {
            let n = string_builder_len(sb);
            to_string(sb)
        }",
    );
}

// 7.10.26t AC3: a closure capturing a handle, and one taking it as an argument.
#[test]
fn a_capturing_closure_is_refused_and_a_parameterised_one_is_not() {
    let captured = "fn f(sb: StringBuilder) -> String {
        let g = |s: String| to_string(push_str(sb, s));
        g(\"x\")
    }";
    assert_eq!(
        refused(captured),
        vec![(
            "E0093",
            "handle `sb` is captured by a closure, in `f`".to_string()
        )]
    );
    accepted(
        "fn f(sb: StringBuilder) -> String {
            let g = |b: StringBuilder| to_string(b);
            g(sb)
        }",
    );
}

// 7.10.26t AC3: a move then a use of the moved handle, and a move alone.
#[test]
fn a_use_after_a_move_is_refused_and_the_moved_name_is_live() {
    let moved = "fn f(a: StringBuilder) -> String {
        let b = a;
        to_string(a)
    }";
    assert_eq!(refused(moved), used_after("a", "f"));
    accepted("fn f(a: StringBuilder) -> String { let b = a; to_string(b) }");
}

// 7.10.26t AC3: a handle-yielding `if` used twice, and once in each arm.
#[test]
fn an_if_yielding_a_handle_is_tracked_and_its_arms_are_alternatives() {
    let twice = "fn f(sb: StringBuilder, c: Bool) -> String {
        let sb2 = if c { push_str(sb, \"a\") } else { sb };
        let s = to_string(sb2);
        to_string(sb2)
    }";
    assert_eq!(refused(twice), used_after("sb2", "f"));
    accepted(
        "fn f(sb: StringBuilder, c: Bool) -> String {
            if c { to_string(sb) } else { to_string(push_str(sb, \"x\")) }
        }",
    );
}

// 7.10.26t AC3 / D6: the handle API declared in one module and misused in
// another is refused, and refused again when the cache serves both.
#[test]
fn a_cross_module_refusal_survives_a_warm_cache() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Mutex::new(BuildCache::new(dir.path(), false).unwrap());
    let root = "mod builder;\nmod user;\n\nfn main() -> Nat { 0 }";
    let user = "use builder::{StringBuilder, push_str, to_string};\n\
                pub fn twice(sb: StringBuilder) -> String {\n\
                    let a = push_str(sb, \"x\");\n\
                    to_string(sb) ++ to_string(a)\n\
                }";
    let files = [
        ("main.tg", root),
        ("builder.tg", BUILDER),
        ("user.tg", user),
    ];
    let cold = run_tree(dir.path(), &files, Some(&cache), 1)
        .err()
        .expect("refused cold");
    let warm = run_tree(dir.path(), &files, Some(&cache), 1)
        .err()
        .expect("refused warm");
    assert_eq!(coded(&cold), used_after("sb", "twice"));
    assert_eq!(coded(&warm), coded(&cold));
    assert_eq!(file_of(&cold[0]), "user.tg");
}

// 13.10.26r AC4: a builder held in a `let mut`, threaded by assignment inside
// one arm of an `if` and consumed once after it, is accepted, cold and warm.
#[test]
fn a_handle_threaded_through_a_join_is_accepted_cold_and_warm() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Mutex::new(BuildCache::new(dir.path(), false).unwrap());
    let body = "fn f(c: Bool) -> String {
        let mut sb = string_builder_new();
        if c { sb = push_str(sb, \"a\"); };
        to_string(sb)
    }";
    let source = format!("{BUILDER}{body}\nfn main() -> Nat {{ 0 }}\n");
    let files = [("main.tg", source.as_str())];
    for run in ["cold", "warm"] {
        if let Err(errors) = run_tree(dir.path(), &files, Some(&cache), 1) {
            panic!("accepted {run}, got {:?}", coded(&errors));
        }
    }
}

// 13.10.26r AC4: two consuming uses through assignments are E0092, at the
// second use.
#[test]
fn two_consuming_uses_through_assignments_are_refused() {
    let twice = "fn f() -> String {
        let mut sb = string_builder_new();
        sb = push_str(sb, \"a\");
        let s = to_string(sb);
        s ++ to_string(sb)
    }";
    assert_eq!(refused(twice), used_after("sb", "f"));
}

// 13.10.26r AC4: a handle in a two-binding join, and one joined beside the
// construct's value, consumed twice after the join, are E0092; once, accepted.
#[test]
fn a_joined_handle_consumed_twice_is_refused_and_once_is_not() {
    let two_bindings = |tail: &str| {
        format!(
            "fn f(c: Bool) -> String {{
                let mut sb = string_builder_new();
                let mut n = 0;
                if c {{ sb = push_str(sb, \"a\"); n = 1; }} else {{ n = 2; }};
                {tail}
            }}"
        )
    };
    let beside_value = |tail: &str| {
        format!(
            "fn f(c: Bool) -> String {{
                let mut sb = string_builder_new();
                let k = if c {{ sb = push_str(sb, \"a\"); 1 }} else {{ 2 }};
                {tail}
            }}"
        )
    };
    let twice = "let s = to_string(sb); s ++ to_string(sb)";
    for shape in [two_bindings, beside_value] {
        assert_eq!(refused(&shape(twice)), used_after("sb", "f"));
        accepted(&shape("to_string(sb)"));
    }
}

/// `body` with `tail` after it, checked through the pipeline twice over one
/// cache: the cold verdict, then the warm one.
fn cold_and_warm(body: &str) -> [Result<(), Vec<(&'static str, String)>>; 2] {
    let dir = tempfile::tempdir().unwrap();
    let cache = Mutex::new(BuildCache::new(dir.path(), false).unwrap());
    let source = format!("{BUILDER}{body}\nfn main() -> Nat {{ 0 }}\n");
    let files = [("main.tg", source.as_str())];
    let mut run = || {
        run_tree(dir.path(), &files, Some(&cache), 1)
            .map(|_| ())
            .map_err(|errors| coded(&errors))
    };
    [run(), run()]
}

// 16.10.26f AC5: a join's *value* is a handle when an arm pairs one — over an
// `if`, over a `match`, and beside a joined handle binding. Consumed twice it
// is E0092 at the second use, cold and warm; consumed once it is accepted.
#[test]
fn a_handle_valued_join_consumed_twice_is_refused_and_once_is_not() {
    let over_if = |tail: &str| {
        format!(
            "fn f(c: Bool) -> String {{
                let mut n = 0;
                let v = if c {{ n = 1; string_builder_new() }} else {{ n = 2; string_builder_new() }};
                {tail}
            }}"
        )
    };
    let over_match = |tail: &str| {
        format!(
            "fn f(k: Nat) -> String {{
                let mut n = 0;
                let v = match k {{ 0 => {{ n = 1; string_builder_new() }}, _ => {{ n = 2; push_str(string_builder_new(), \"y\") }} }};
                {tail}
            }}"
        )
    };
    let beside_binding = |tail: &str| {
        format!(
            "fn f(c: Bool) -> String {{
                let mut a = string_builder_new();
                let v = if c {{ a = push_str(a, \"x\"); string_builder_new() }} else {{ string_builder_new() }};
                let t = to_string(a);
                {tail}
            }}"
        )
    };
    let twice = "let s = to_string(v); s ++ to_string(v)";
    for shape in [over_if, over_match, beside_binding] {
        let [cold, warm] = cold_and_warm(&shape(twice));
        assert_eq!(cold, Err(used_after("v", "f")));
        assert_eq!(warm, cold);
        assert_eq!(cold_and_warm(&shape("to_string(v)")), [Ok(()), Ok(())]);
    }
}
