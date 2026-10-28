//! The write-cover rule (ADR 9.10.26n D3, AC 2), over injected Core terms.
//! The embedded sources are checked beside the core's other rules, in
//! `native_core/tests.rs`.

use super::adopt_tests::call;
use super::cover::uncovered_buffers;
use super::{check_core_rules, write_cover_refusal, CoreRuleViolation, DefinitionFacts};
use albite_core::terms::Term;
use albite_core::types::Type;

fn var(name: &str) -> Term {
    Term::Var(name.to_string())
}

fn len(name: &str) -> Term {
    Term::StrLen(Box::new(var(name)))
}

fn add(left: Term, right: Term) -> Term {
    Term::NatAdd(Box::new(left), Box::new(right))
}

fn lambdas(params: &[&str], body: Term) -> Term {
    params.iter().rev().fold(body, |body, param| {
        Term::Lambda(param.to_string(), Type::String, Box::new(body))
    })
}

fn let_in(name: &str, definition: Term, body: Term) -> Term {
    Term::Let(
        name.to_string(),
        Type::Nat,
        Box::new(definition),
        Box::new(body),
    )
}

fn open(size: Term) -> Term {
    call("bytes::bytes_new_uninit", vec![size])
}

fn blit(buffer: &str, offset: Term, text: Term) -> Term {
    call("bytes::bytes_blit_string", vec![var(buffer), offset, text])
}

fn unchecked(buffer: &str) -> Term {
    call("bytes::bytes_into_string_unchecked", vec![var(buffer)])
}

/// `let b = open(size); let b = blit(b, o, s); …; unchecked(b)`, shadowing
/// `b` at each link as the real concat does.
fn chain(size: Term, copies: Vec<(Term, &str)>) -> Term {
    let tail = copies
        .into_iter()
        .rev()
        .fold(unchecked("b"), |body, (offset, text)| {
            let_in("b", blit("b", offset, var(text)), body)
        });
    let_in("b", open(size), tail)
}

/// `string_concat`'s real shape: `let n = len left; <chain>`.
fn concat(body: Term) -> Term {
    lambdas(&["left", "right"], let_in("n", len("left"), body))
}

fn real_concat() -> Term {
    concat(chain(
        add(var("n"), len("right")),
        vec![(Term::Zero, "left"), (var("n"), "right")],
    ))
}

/// The violations `term`, defined as `name`, gets from the whole rule set.
fn violations(name: &str, term: &Term) -> Vec<CoreRuleViolation> {
    let facts = DefinitionFacts {
        name: name.to_string(),
        uncovered_buffers: uncovered_buffers(name, term),
        ..DefinitionFacts::default()
    };
    check_core_rules(&[facts], &[], &[])
}

/// Exactly the write-cover rule's refusal, naming `name`.
fn assert_uncovered(name: &str, term: &Term) {
    let found = violations(name, term);
    assert!(!found.is_empty(), "{name}: accepted");
    for violation in &found {
        match violation {
            CoreRuleViolation::UncoveredBuffer { definition, .. } => {
                assert_eq!(definition, name);
            }
            other => panic!("{name}: not UncoveredBuffer: {other}"),
        }
    }
}

// 9.10.26n AC 2: the concat's own shape, and a three-copy chain.
#[test]
fn the_real_concat_and_a_three_copy_chain_are_accepted() {
    assert_eq!(violations("string::string_concat", &real_concat()), vec![]);
    let three = lambdas(
        &["x", "y", "z"],
        chain(
            add(add(len("x"), len("y")), len("z")),
            vec![
                (Term::NatLit(0), "x"),
                (len("x"), "y"),
                (add(len("y"), len("x")), "z"),
            ],
        ),
    );
    assert_eq!(violations("string_concat", &three), vec![]);
}

// 9.10.26n AC 2: an overlap and a trailing gap — the second copy at `o₁`.
#[test]
fn a_second_copy_at_the_first_offset_is_refused() {
    let term = concat(chain(
        add(var("n"), len("right")),
        vec![(Term::Zero, "left"), (Term::Zero, "right")],
    ));
    assert_uncovered("string_concat", &term);
}

// 9.10.26n AC 2: a trailing gap — the size plus one.
#[test]
fn a_size_one_past_the_copies_is_refused() {
    let term = concat(chain(
        add(add(var("n"), len("right")), Term::NatLit(1)),
        vec![(Term::Zero, "left"), (var("n"), "right")],
    ));
    assert_uncovered("string_concat", &term);
    // Binders are numbered in binding order: `left` #0, `right` #1.
    assert_eq!(
        uncovered_buffers("string_concat", &term),
        vec![
            "the copies end at 0 + len(#0) + len(#1), not the range's end \
             1 + len(#0) + len(#1)"
                .to_string()
        ]
    );
    let doubled = lambdas(
        &["x"],
        chain(add(len("x"), len("x")), vec![(Term::Zero, "x")]),
    );
    assert_eq!(
        uncovered_buffers("string_concat", &doubled),
        vec!["the copies end at 0 + len(#0), not the range's end 0 + 2·len(#0)".to_string()]
    );
}

// 9.10.26n AC 2: a pure overlap with a correct end — `(0, a)`, `(len a, b)`,
// `(len a, c)` over `len a + len c`.
#[test]
fn a_pure_overlap_with_a_correct_end_is_refused() {
    let term = lambdas(
        &["a", "b2", "c"],
        chain(
            add(len("a"), len("c")),
            vec![(Term::Zero, "a"), (len("a"), "b2"), (len("a"), "c")],
        ),
    );
    assert_uncovered("string_concat", &term);
}

// 9.10.26n AC 2: a `bytes_get` or a `bytes_len` between the opening call
// and the conversion.
#[test]
fn a_read_before_the_conversion_is_refused() {
    for read in ["bytes::bytes_get", "bytes::bytes_len"] {
        let args = if read.ends_with("get") {
            vec![var("b"), Term::Zero]
        } else {
            vec![var("b")]
        };
        let after_copies = let_in(
            "b",
            blit("b", Term::Zero, var("left")),
            let_in("v", call(read, args), unchecked("b")),
        );
        let term = concat(let_in("b", open(len("left")), after_copies));
        assert_uncovered("string_concat", &term);
    }
}

// 9.10.26n AC 2: a branch between the opening call and the conversion.
#[test]
fn a_branch_before_the_conversion_is_refused() {
    let arm = let_in("b", blit("b", Term::Zero, var("left")), unchecked("b"));
    let branch = Term::If(Box::new(Term::True), Box::new(arm.clone()), Box::new(arm));
    let term = concat(let_in("b", open(len("left")), branch));
    assert_uncovered("string_concat", &term);
}

// 9.10.26n AC 2: the checked `bytes_into_string` as the final conversion.
#[test]
fn the_checked_conversion_is_refused() {
    let checked = call("bytes::bytes_into_string", vec![var("b")]);
    let term = concat(let_in(
        "b",
        open(len("left")),
        let_in("b", blit("b", Term::Zero, var("left")), checked),
    ));
    assert_uncovered("string_concat", &term);
}

// 9.10.26n AC 2: a string argument that is not a variable.
#[test]
fn a_copy_of_a_non_variable_string_is_refused() {
    let literal = Term::StringLit("left".to_string());
    let term = concat(let_in(
        "b",
        open(len("left")),
        let_in("b", blit("b", Term::Zero, literal), unchecked("b")),
    ));
    assert_uncovered("string_concat", &term);
}

// 9.10.26n AC 2: a stale alias — `let n = len left; let left = other; …
// blit(b, n, left)`: `n` is the old `left`'s length, not the new one's.
#[test]
fn a_stale_alias_is_refused() {
    let body = let_in(
        "left",
        var("other"),
        chain(
            add(len("left"), var("n")),
            vec![(Term::Zero, "left"), (var("n"), "left")],
        ),
    );
    let term = lambdas(&["left", "other"], let_in("n", len("left"), body));
    assert_uncovered("string_concat", &term);
    // Not inert: with no rebinding the same copies tile `len left + n`.
    let fresh = lambdas(
        &["left", "other"],
        let_in(
            "n",
            len("left"),
            chain(
                add(len("left"), var("n")),
                vec![(Term::Zero, "left"), (var("n"), "left")],
            ),
        ),
    );
    assert_eq!(violations("string_concat", &fresh), vec![]);
}

// 9.10.26n AC 2: a chain link that skips a binder.
#[test]
fn a_link_that_skips_a_binder_is_refused() {
    let term = concat(let_in(
        "b0",
        open(add(var("n"), len("right"))),
        let_in(
            "b1",
            blit("b0", Term::Zero, var("left")),
            let_in("b2", blit("b0", var("n"), var("right")), unchecked("b2")),
        ),
    ));
    assert_uncovered("string_concat", &term);
}

// 9.10.26n AC 2: the extern outside its wrapper and stub.
#[test]
fn the_extern_is_named_only_in_its_wrapper_and_stub() {
    let extern_call = Term::ExternCall("__c_tg_bytes_new_uninit".to_string(), vec![var("n")]);
    let wrapper = lambdas(&["n"], extern_call.clone());
    assert_eq!(violations("bytes::bytes_new_uninit", &wrapper), vec![]);
    assert_eq!(violations("tg_bytes_new_uninit", &wrapper), vec![]);
    assert_uncovered("string_concat", &concat(extern_call));
    let by_stub = concat(call("bytes::tg_bytes_new_uninit", vec![var("n")]));
    assert_uncovered("string_concat", &by_stub);
}

// 9.10.26n AC 2: the wrapper called, or named as a value, from a definition
// no export row names; and as a value even in one that does.
#[test]
fn the_wrapper_is_called_only_in_an_export_row_and_never_as_a_value() {
    assert_uncovered("string::helper", &real_concat());
    let as_value = concat(call(
        "string::apply_to",
        vec![Term::Global("bytes::bytes_new_uninit".to_string())],
    ));
    assert_uncovered("string::helper", &as_value);
    assert_uncovered("string_concat", &as_value);
    let bare_call = concat(call("string::consume", vec![open(len("left"))]));
    assert_uncovered("string_concat", &bare_call);
}

// 6.10.26n's owned shape: the grow opens `[len left, len left + len right)`,
// and one copy at `n` tiles it.
#[test]
fn the_owned_grow_is_accepted_and_a_gap_in_it_refused() {
    let grown = |offset: Term| {
        let grow = call(
            "bytes::bytes_from_string_grown",
            vec![var("left"), len("right")],
        );
        let_in(
            "b",
            grow,
            let_in("b", blit("b", offset, var("right")), unchecked("b")),
        )
    };
    let owned = |offset: Term| {
        let fresh = call("string::string_concat", vec![var("left"), var("right")]);
        concat(Term::If(
            Box::new(Term::True),
            Box::new(fresh),
            Box::new(grown(offset)),
        ))
    };
    assert_eq!(violations("string_concat_owned", &owned(var("n"))), vec![]);
    assert_uncovered("string_concat_owned", &owned(Term::Zero));
}

// The release-build refusal names only the write-cover rule's violations.
#[test]
fn the_release_refusal_names_only_uncovered_buffers() {
    let other = CoreRuleViolation::Generic {
        definition: "g".to_string(),
    };
    assert_eq!(write_cover_refusal(std::slice::from_ref(&other)), None);
    let uncovered = CoreRuleViolation::UncoveredBuffer {
        definition: "string_concat".to_string(),
        reason: "a gap".to_string(),
    };
    let refusal = write_cover_refusal(&[other, uncovered.clone(), uncovered]).expect("refused");
    assert_eq!(refusal.lines().count(), 2);
    assert!(
        refusal.contains("string_concat: an unfilled buffer"),
        "{refusal}"
    );
    assert!(refusal.contains("UncoveredBuffer"), "{refusal}");
    assert!(refusal.ends_with("a gap"), "{refusal}");
}
