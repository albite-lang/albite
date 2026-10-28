//! ADR 4.10.26g AC 2b — `rewrite`'s split (D4): the Core back end,
//! `rewrite_by_equation`, builds what the front end builds through it.

use albite_core::{Context, Type};

use super::{add, eq_nat, var};
use crate::ast::{Expr, Ident, Path, RewriteRule};
use crate::elaborate::tactics::rewrite::{Equation, EquationSite, RuleStep};
use crate::elaborate::tactics::state::{Partial, ProofState};
use crate::elaborate::Elaborator;
use crate::span::Span;

// ADR 4.10.26g AC2b: the split changes nothing — the back end, handed the
// equation the front end would elaborate, builds the same `Subst` and the
// same new goal, forward and reversed.
#[test]
fn rewrite_by_equation_builds_the_same_term() {
    let span = Span::new(3, 4);
    let locals = vec![
        ("a".to_string(), Type::Nat),
        ("b".to_string(), Type::Nat),
        ("h".to_string(), eq_nat(var("a"), var("b"))),
    ];
    for (reverse, target) in [
        (false, eq_nat(add(var("a"), var("a")), var("b"))),
        (true, eq_nat(var("a"), add(var("b"), var("b")))),
    ] {
        let rule = RewriteRule {
            reverse,
            rule: Expr::Path(Path::simple(Ident::new("h", span))),
            span,
        };
        let mut ctx = Context::new();
        let mut elaborator = Elaborator::new(&mut ctx);
        let mut front_state = ProofState::new(Type::Unit);
        let front_goal = front_state.fresh_meta(locals.clone(), target.clone());
        let step = RuleStep {
            tactic: "rewrite",
            rule: &rule,
            position: 1,
        };
        let front = elaborator
            .run_rewrite_rule(step, &mut front_state, front_goal)
            .expect("the front end rewrites");
        let mut back_state = ProofState::new(Type::Unit);
        let back_goal = back_state.fresh_meta(locals.clone(), target);
        let equation = Equation {
            proof: var("h"),
            base: Type::Nat,
            left: var("a"),
            right: var("b"),
        };
        let at = EquationSite {
            tactic: "rewrite",
            position: 1,
            span,
            reverse,
        };
        let back = elaborator
            .rewrite_by_equation(&mut back_state, back_goal, equation, at)
            .expect("the back end rewrites");
        assert_eq!(front.template.strip_spans(), back.template.strip_spans());
        let new_goal =
            |state: &ProofState, partial: &Partial| state.metas[partial.holes[0].1].target.clone();
        assert_eq!(new_goal(&front_state, &front), new_goal(&back_state, &back));
    }
}
