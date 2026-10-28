//! `cases` through the elaborator (ADR 3.10.26f AC 2): the Core it emits,
//! its trace, and each of D4's refusals with its code and `why`.
//!
//! `elab_ok` runs the elaborator, not the per-module kernel boundary, so the
//! kernel's verdict is AC 3's, over `src/compiler/test_tactic_cases.tg`.

use albite_core::Term;

use crate::elaborate::error::TacticErrorKind;
use crate::elaborate::tactics::tests::tactic_error;
use crate::elaborate::tests::{elab_err, elab_ok};

/// The types every source below declares.
const TYPES: &str = "type Opt = None | Some(Nat)
type Lst = Nil | Cons(Nat, Lst)
type Tri = Lo | Mid(Nat) | Hi
type Pt = { x: Nat, y: Nat }
fn some_one() -> Opt { Some(1) }
";

fn with_types(theorem: &str) -> String {
    format!("{TYPES}{theorem}")
}

/// E0082's `why`, asserting the tactic it names is `cases`.
fn refusal_why(theorem: &str) -> String {
    let source = with_types(theorem);
    let codes: Vec<&str> = elab_err(&source).iter().map(|e| e.kind.code()).collect();
    assert!(codes.contains(&"E0082"), "{codes:?}");
    match tactic_error(&source) {
        TacticErrorKind::TacticNotApplicable { tactic, why, .. } => {
            assert_eq!(tactic, "cases");
            why
        }
        other => panic!("expected E0082, got {other:?}"),
    }
}

// ADR 3.10.26f AC2: the emitted Core is the annotated split over the
// parameter, with one `refl` per arm.
#[test]
fn cases_emits_an_annotated_split() {
    let defs = elab_ok(&with_types(
        "theorem tri_refl(t: Tri) -> Eq<Tri, t, t> by { cases t { Hi => refl, Lo => refl, Mid(n) => refl } }",
    ));
    let def = defs
        .iter()
        .find(|d| d.name == "tri_refl")
        .expect("the theorem");
    let Term::Lambda(_, _, body) = &def.term.term else {
        panic!("expected the parameter's λ, got {}", def.term.term);
    };
    let Term::Annot(split, _) = &**body else {
        panic!("expected the annotated split, got {body}");
    };
    let Term::AdtMatch(scrutinee, arms) = &**split else {
        panic!("expected an adt_match, got {split}");
    };
    assert_eq!(**scrutinee, Term::var("t"));
    let indices: Vec<usize> = arms.iter().map(|(index, _, _)| *index).collect();
    assert_eq!(indices, [2, 0, 1], "the arms keep their source order");
}

// ADR 3.10.26f AC2: a recursive type is split through `unfold`, and an arm
// may hold a sequence.
#[test]
fn a_recursive_scrutinee_is_unfolded() {
    let defs = elab_ok(&with_types(
        "theorem lst_refl(l: Lst) -> Eq<Lst, l, l> by { cases l { Nil => refl, Cons(h, t) => { refl } } }",
    ));
    let def = defs
        .iter()
        .find(|d| d.name == "lst_refl")
        .expect("the theorem");
    let rendered = def.term.term.to_string();
    assert!(rendered.contains("case (unfold ["), "{rendered}");
}

// ADR 3.10.26f AC2
#[test]
fn a_compound_scrutinee_is_refused() {
    let why = refusal_why(
        "theorem t(o: Opt) -> Eq<Opt, o, o> by { cases some_one() { None => refl, Some(x) => refl } }",
    );
    assert_eq!(why, "the scrutinee is not a local");
}

// ADR 3.10.26f AC2
#[test]
fn a_global_scrutinee_is_refused() {
    let why = refusal_why(
        "theorem t(o: Opt) -> Eq<Opt, o, o> by { cases some_one { None => refl, Some(x) => refl } }",
    );
    assert!(why.contains("is a global"), "{why}");
}

// ADR 3.10.26f AC2
#[test]
fn a_nat_scrutinee_points_at_induction() {
    let why = refusal_why("theorem t(n: Nat) -> Eq<Nat, n, n> by { cases n { Zero => refl } }");
    assert!(why.contains("`induction`"), "{why}");
}

// ADR 3.10.26f AC2
#[test]
fn a_bool_scrutinee_is_refused() {
    let why = refusal_why("theorem t(b: Bool) -> Eq<Bool, b, b> by { cases b { True => refl } }");
    assert!(why.contains("`Bool`"), "{why}");
}

// ADR 3.10.26f AC2: the record is named as the source spells it, never by
// its collection-time `@` name.
#[test]
fn a_record_scrutinee_is_refused() {
    let why = refusal_why("theorem t(p: Pt) -> Eq<Pt, p, p> by { cases p { Pt(a, b) => refl } }");
    assert_eq!(
        why,
        "`Pt` is a record: it has one constructor, reached by projection"
    );
}

// ADR 3.10.26f AC2: the twin's tenth case — a false lemma is refused by the
// arm that cannot be closed, with that arm tactic's own code.
#[test]
fn a_false_lemma_is_refused_in_its_arm() {
    let source = with_types(
        "theorem t(o: Opt) -> Eq<Opt, o, None> by { cases o { None => refl, Some(x) => refl } }",
    );
    let codes: Vec<&str> = elab_err(&source).iter().map(|e| e.kind.code()).collect();
    assert_eq!(codes, ["E0071"]);
}

// ADR 3.10.26f AC2
#[test]
fn a_missing_arm_is_refused() {
    let why =
        refusal_why("theorem t(t: Tri) -> Eq<Tri, t, t> by { cases t { Lo => refl, Hi => refl } }");
    assert_eq!(why, "no arm for `Mid`");
}

// ADR 3.10.26f AC2
#[test]
fn a_duplicate_arm_is_refused() {
    let why = refusal_why(
        "theorem t(o: Opt) -> Eq<Opt, o, o> by { cases o { None => refl, Some(x) => refl, Some(y) => refl } }",
    );
    assert_eq!(why, "`Some` has two arms");
}

// ADR 3.10.26f AC2: unknown, and a constructor of another type.
#[test]
fn an_unknown_constructor_is_refused() {
    let why = refusal_why(
        "theorem t(o: Opt) -> Eq<Opt, o, o> by { cases o { None => refl, Many(x) => refl } }",
    );
    assert_eq!(why, "no constructor `Many`");
    let why = refusal_why(
        "theorem t(o: Opt) -> Eq<Opt, o, o> by { cases o { None => refl, Lo => refl } }",
    );
    assert!(why.starts_with("`Lo` is not a constructor of"), "{why}");
}

// ADR 3.10.26f AC2
#[test]
fn a_wrong_binder_count_is_refused() {
    let why = refusal_why(
        "theorem t(o: Opt) -> Eq<Opt, o, o> by { cases o { None => refl, Some(x, y) => refl } }",
    );
    assert_eq!(why, "constructor `Some` has 1 field(s), the arm binds 2");
}

// ADR 3.10.26f AC2: a field may not capture a name the goal or the scope
// already uses (3.10.26o D1's no-capture rule, refused here first).
#[test]
fn a_field_binder_that_would_capture_is_refused() {
    let why = refusal_why(
        "theorem t(o: Opt) -> Eq<Opt, o, o> by { cases o { None => refl, Some(o) => refl } }",
    );
    assert_eq!(why, "`o` is already bound");
    let why = refusal_why(
        "theorem t(l: Lst) -> Eq<Lst, l, l> by { cases l { Nil => refl, Cons(h, h) => refl } }",
    );
    assert_eq!(why, "`h` is bound twice in one arm");
}

// ADR 3.10.26f AC2: an arm left open is E0083 at the arm, and an arm's extra
// tactic finds no goal rather than reaching the next arm's.
#[test]
fn an_open_arm_is_e0083() {
    let source = with_types(
        "theorem t(o: Opt) -> Eq<Opt, o, o> by { cases o { None => refl, Some(x) => {} } }",
    );
    let errors = elab_err(&source);
    let open = errors
        .iter()
        .find(|e| e.kind.code() == "E0083")
        .unwrap_or_else(|| panic!("no E0083 in {errors:?}"));
    let start = open.span.start as usize;
    assert!(
        source[start..].starts_with("Some(x) => {}"),
        "{}",
        &source[start..]
    );
    let source = with_types(
        "theorem t(o: Opt) -> Eq<Opt, o, o> by { cases o { None => { refl; refl }, Some(x) => refl } }",
    );
    match tactic_error(&source) {
        TacticErrorKind::TacticNotApplicable { tactic, why, .. } => {
            assert_eq!(tactic, "refl");
            assert_eq!(why, crate::elaborate::error::NO_GOAL_LEFT);
        }
        other => panic!("expected E0082, got {other:?}"),
    }
}

// ADR 3.10.26f AC2: a tactic after the split runs on the goals the split
// left, which are none.
#[test]
fn a_tactic_after_a_closed_split_finds_no_goal() {
    let source = with_types(
        "theorem t(o: Opt) -> Eq<Opt, o, o> by { cases o { None => refl, Some(x) => refl }; refl }",
    );
    match tactic_error(&source) {
        TacticErrorKind::TacticNotApplicable { tactic, why, .. } => {
            assert_eq!(tactic, "refl");
            assert_eq!(why, crate::elaborate::error::NO_GOAL_LEFT);
        }
        other => panic!("expected E0082, got {other:?}"),
    }
}

// ADR 3.10.26f AC2: a parenthesised local is still a local, and a name in no
// scope is refused as such, not as a global.
#[test]
fn the_scrutinee_is_resolved_through_parens_and_scope() {
    elab_ok(&with_types(
        "theorem t(o: Opt) -> Eq<Opt, o, o> by { cases (o) { None => refl, Some(x) => refl } }",
    ));
    let why = refusal_why(
        "theorem t(o: Opt) -> Eq<Opt, o, o> by { cases q { None => refl, Some(x) => refl } }",
    );
    assert_eq!(why, "`q` is not a variable in scope");
}

// ADR 3.10.26f AC2: a qualified path is not a local, even when its last
// segment names one; an `intro`'d local splits like a parameter.
#[test]
fn only_a_plain_local_is_a_scrutinee() {
    let why = refusal_why(
        "theorem t(o: Opt) -> Eq<Opt, o, o> by { cases Opt::o { None => refl, Some(x) => refl } }",
    );
    assert_eq!(why, "the scrutinee is not a local");
    elab_ok(&with_types(
        "theorem t(n: Nat) -> Opt -> Eq<Nat, n, n> by { intro o; cases o { None => refl, Some(x) => refl } }",
    ));
}

// ADR 3.10.26f AC2: a type parameter is not a record, and not a data type.
#[test]
fn a_type_parameter_scrutinee_is_refused() {
    let why = refusal_why("theorem t<T>(v: T) -> Eq<T, v, v> by { cases v { None => refl } }");
    assert_eq!(why, "`T` is not a user data type");
}

// ADR 3.10.26f AC2: each route by which a field could capture — a parameter
// other than the scrutinee, an `intro`'d local, a name the goal mentions.
#[test]
fn a_field_may_not_reuse_a_bound_or_mentioned_name() {
    let why = refusal_why(
        "theorem t(n: Nat, o: Opt) -> Eq<Opt, o, o> by { cases o { None => refl, Some(n) => refl } }",
    );
    assert_eq!(why, "`n` is already bound");
    let why = refusal_why(
        "theorem t(o: Opt) -> Nat -> Eq<Opt, o, o> by { intro k; cases o { None => refl, Some(k) => refl } }",
    );
    assert_eq!(why, "`k` is already bound");
    let why = refusal_why(
        "theorem t(o: Opt) -> Eq<Opt, o, some_one()> by { cases o { None => refl, Some(some_one) => refl } }",
    );
    assert_eq!(why, "`some_one` is already bound");
}

// ADR 4.10.26d AC3: every arm names a constructor of one other type, whose
// arms agree with each other and whose indices all fit the scrutinee's
// shape, so only the owner check against the scrutinee refuses it (P0.2:
// accepted before, with `k` bound at the real payload type).
#[test]
fn a_constructor_of_another_type_is_refused_by_cases() {
    let why = refusal_why(
        "type OptP = NoP(Bool) | YesP(Nat)
type OtherP = AP(Nat) | BP(Bool)
theorem t(o: OptP) -> Eq<OptP, o, o> by { cases o { AP(k) => refl, BP(b) => refl } }",
    );
    assert_eq!(why, "`AP` is not a constructor of `(Bool + Nat)`");
}

// ADR 4.10.26d AC3: a same-shaped type follows the typed-`let` rule here too.
#[test]
fn a_same_shaped_type_is_admitted_by_cases() {
    elab_ok(&with_types(
        "type SameOpt = SNone | SSome(Nat)
theorem t(o: Opt) -> Eq<Opt, o, o> by { cases o { SNone => refl, SSome(k) => refl } }",
    ));
}
