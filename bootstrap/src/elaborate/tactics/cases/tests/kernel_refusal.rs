//! The kernel's side of `cases` (ADR 3.10.26f): AC 2's hand-broken split,
//! refused through S1's discharge and named `cases`, and AC 3's corpus,
//! admitted through the build's own gate.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use albite_core::{SpannedTerm, Term, TermSpan, Type};

use super::{opt, shape_of};
use crate::elaborate::error::ElabErrorKind;
use crate::elaborate::kernel::{
    discharge_local, KernelObligation, ObligationOrigin, RecordPresentation,
};
use crate::elaborate::tactics::cases::split::{arm_body, arm_goal, split_term, SplitArm};
use crate::elaborate::tactics::state::{hole_name, Partial, ProofState};
use crate::elaborate::tactics::{TacticStep, TacticTrace};
use crate::elaborate::CoreDef;
use crate::span::Span;

/// `case o of inl u => None | inr x => Some(x)`: the identity on `Opt`,
/// written into the statement so each arm closes by ι alone.
fn rebuild_opt(scrutinee: Term) -> Term {
    Term::case(
        scrutinee,
        "u",
        Term::inl(opt(), Term::Unit),
        "x",
        Term::inr(opt(), Term::var("x")),
    )
}

// ADR 3.10.26f AC2: a hand-broken `split.rs` output — the `Some` arm's goal
// built at `None`'s index — passes every arm's own check and is refused by
// the kernel as a whole: E0081, attributed to the `cases` step that built
// the arms, never to an arm's `refl`.
#[test]
fn a_kernel_refusal_names_the_arm() {
    let goal = Type::eq(opt(), rebuild_opt(Term::var("o")), Term::var("o"));
    let shape = shape_of(&opt());
    let mut state = ProofState::new(goal.clone());
    let none_goal = arm_goal("o", &goal, &shape, 0, &[]);
    let broken_some_goal = arm_goal("o", &goal, &shape, 0, &[]);
    let none_hole = state.fresh_meta(Vec::new(), none_goal);
    let some_hole = state.fresh_meta(vec![("x".into(), Type::Nat)], broken_some_goal);
    let template = split_term(
        "o",
        &shape,
        vec![
            SplitArm {
                index: 0,
                binder: "%b0".into(),
                body: arm_body("%b0", &[], &hole_name(none_hole)),
            },
            SplitArm {
                index: 1,
                binder: "%b1".into(),
                body: arm_body("%b1", &[("x".into(), Type::Nat)], &hole_name(some_hole)),
            },
        ],
        &goal,
    )
    .unwrap();
    let cases_span = Span::new(10, 20);
    let mut steps = vec![TacticStep {
        tactic: "cases",
        span: cases_span,
        goal: 0,
        exact_sorry: false,
        citation: None,
    }];
    state.refine(
        0,
        Partial {
            template,
            holes: vec![
                (hole_name(none_hole), none_hole),
                (hole_name(some_hole), some_hole),
            ],
        },
    );
    for (hole, at) in [(none_hole, 30), (some_hole, 40)] {
        let proof = Term::refl(opt(), Term::inl(opt(), Term::Unit));
        state.refine(hole, Partial::closed(proof));
        steps.push(TacticStep {
            tactic: "refl",
            span: Span::new(at, at + 4),
            goal: hole,
            exact_sorry: false,
            citation: None,
        });
    }
    let trace = TacticTrace {
        state,
        steps,
        value_params: vec![("o".into(), opt())],
        type_params: Vec::new(),
        block_span: Span::new(0, 50),
    };
    let term = trace.assemble(&BTreeMap::new()).expect("a finished trace");
    let span = Span::new(0, 60);
    let def = CoreDef {
        name: "broken".into(),
        ty: Type::pi_or_arrow("o", opt(), goal),
        term: SpannedTerm::new(term, TermSpan::new(span.start, span.end)),
        span,
    };
    let obligation = KernelObligation {
        def: "broken".into(),
        span,
        module: None,
        origin: ObligationOrigin::Tactics(trace),
    };
    let errors = discharge_local(
        &[def],
        &HashMap::new(),
        &RecordPresentation::EMPTY,
        vec![obligation],
    );
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].kind.code(), "E0081");
    let ElabErrorKind::KernelRefusedProof { tactic, cause, .. } = &errors[0].kind else {
        panic!("expected E0081, got {:?}", errors[0].kind);
    };
    assert_eq!(
        tactic.as_ref().map(|(name, at)| (name.as_str(), *at)),
        Some(("cases", cases_span))
    );
    assert!(!cause.is_empty());
}

// ADR 3.10.26f AC3: the corpus builds cold through the module-tree walk,
// whose admission gate discharges every tactic proof in the kernel — a
// refusal there is E0081 and an `Err` here — and holds all five lemmas.
#[test]
fn the_cases_corpus_passes_the_kernel_gate() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/compiler/test_tactic_cases.tg");
    let project = crate::driver::elaborate_project_cold(&corpus, 20)
        .unwrap_or_else(|errors| panic!("the corpus is refused: {errors:?}"));
    for lemma in ["opt_id", "maybe_id", "lst_id", "tri_id", "shape_id"] {
        assert!(
            project.defs.iter().any(|def| def.name == lemma),
            "{lemma} is missing"
        );
    }
}
