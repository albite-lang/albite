//! The `by` block parser (ADR 2.10.26c D1).
//!
//! ```text
//! tactic_block := '{' ( tactic ( ';' tactic )* ';'? )? '}'
//! tactic       := 'intro' IDENT | 'exact' expr | 'apply' expr | 'refl'
//!               | 'rewrite' '[' rule ( ',' rule )* ']' | 'induction' IDENT
//!               | 'cases' expr '{' arm ( ',' arm )* ','? '}'
//!               | 'cases' expr '{' pair_arm ','? '}'
//!               | 'simp' '[' rule ( ',' rule )* ']' | 'ring' | 'omega'
//!               | 'exists' expr | 'left' | 'right'
//! rule         := ( '<' '-' )? expr
//! arm          := IDENT ( '(' binder ( ',' binder )* ')' )? '=>' arm_body
//! pair_arm     := '(' binder ',' binder ')' '=>' arm_body
//! binder       := IDENT | '_'
//! arm_body     := tactic | '{' ( tactic ( ';' tactic )* ';'? )? '}'
//! ```
//!
//! Tactic names are matched by the identifier's text, not by token kind, so
//! `intro`, `exact`, `apply` and `rewrite` stay ordinary identifiers everywhere
//! else (`fn apply(…)` still parses). `refl` is already the reserved `Refl`
//! token. `rewrite` cannot be a keyword, because the self-host has a
//! `harness/rewrite.tg` (ADR 2.10.26e D1), nor `induction`, because it has
//! `elab/exprs/forms/induction.tg` (ADR 3.10.26e D1). `cases` is contextual
//! too (ADR 3.10.26f D1); its scrutinee is an expression so that the
//! elaborator, not the parser, refuses a compound one. `simp` is contextual
//! too (ADR 4.10.26e D1) and takes `rewrite`'s rule list, which it requires.
//! `ring` is contextual (ADR 4.10.26g D1) and takes nothing, and so is
//! `omega` (ADR 6.10.26f D1), and so are `left` and `right` (ADR 7.10.26l
//! D2), which pick a disjunction's side. `exists` is the
//! reserved proof keyword, matched by token kind like `refl`, so no name can
//! collide with it (ADR 6.10.26e D4). A `cases` whose one arm starts with `(`
//! is the pair arm, which unpacks an `exists` local (ADR 6.10.26e D6).
//!
//! A rule's `<-` lexes as `Lt` then `Minus`. No expression starts with `<`, so
//! the pair at a rule's start is unambiguous and the lexer does not change.

use crate::ast::{CasesArm, Ident, RewriteRule, Tactic, TacticBlock};
use crate::error::ParseErrorKind;
use crate::span::{Span, Spanned};
use crate::token::TokenKind;

use super::Parser;

/// What an unknown step reports. No new code: it is a parse error like any
/// other `Expected`.
pub(crate) const EXPECTED_A_TACTIC: &str =
    "a tactic: intro, exact, apply, refl, rewrite, induction, cases, simp, ring, omega, left, right or exists";

/// What a pair arm that is not `(binder, binder)` reports: an `exists` has
/// exactly two parts (ADR 6.10.26e D6).
pub(crate) const EXPECTED_A_PAIR_ARM: &str = "a pair arm: `(witness, evidence) => tactic`";

/// What `ring` followed by anything but the step's end reports: it takes no
/// arguments (ADR 4.10.26g D1).
pub(crate) const EXPECTED_RING_ALONE: &str = "`;` or `}` after `ring`, which takes no arguments";

/// What `omega` followed by anything but the step's end reports: it takes
/// no arguments (ADR 6.10.26f D1).
pub(crate) const EXPECTED_OMEGA_ALONE: &str = "`;` or `}` after `omega`, which takes no arguments";

/// What `left` followed by anything but the step's end reports: it takes no
/// arguments (ADR 7.10.26l D2).
pub(crate) const EXPECTED_LEFT_ALONE: &str = "`;` or `}` after `left`, which takes no arguments";

/// What `right` followed by anything but the step's end reports (ADR
/// 7.10.26l D2).
pub(crate) const EXPECTED_RIGHT_ALONE: &str = "`;` or `}` after `right`, which takes no arguments";

/// What `cases` with no arm reports (ADR 3.10.26f D1).
pub(crate) const EXPECTED_A_CASES_ARM: &str = "a `cases` arm: `Constructor(binders) => tactic`";

/// What an arm binder that is not a name or `_` reports: nested patterns and
/// literals are not arms (ADR 3.10.26f D1).
pub(crate) const EXPECTED_A_CASES_BINDER: &str = "a binder name or `_` in a `cases` arm";

/// What `induction` over anything but one variable reports (ADR 3.10.26e D1).
pub(crate) const EXPECTED_AN_INDUCTION_TARGET: &str = "a variable: `induction <variable>`";

/// What `rewrite` without its bracketed list reports (ADR 2.10.26e D1).
pub(crate) const EXPECTED_A_RULE_LIST: &str = "a bracketed rule list: `rewrite [h]`";

/// What `simp` with no list, or an empty one, reports (ADR 4.10.26e D1):
/// there is no global lemma database to fall back on.
pub(crate) const EXPECTED_A_SIMP_LIST: &str =
    "a lemma list: `simp` takes an explicit lemma list, `simp [h]`";

/// What a `<` at a rule's start that is not `<-` reports.
pub(crate) const EXPECTED_A_REVERSE_ARROW: &str = "`<-` before a rule";

impl Parser<'_> {
    /// Parse `by { … }`, the `by` token current.
    pub(super) fn parse_tactic_block(&mut self) -> Option<TacticBlock> {
        let start = self.current_span().start;
        self.expect(TokenKind::By)?;
        self.expect(TokenKind::LBrace)?;
        let mut tactics = Vec::new();
        while !self.check(TokenKind::RBrace) {
            tactics.push(self.parse_tactic()?);
            if !self.eat(TokenKind::Semi) {
                break;
            }
        }
        let end = self.expect(TokenKind::RBrace)?.end;
        Some(TacticBlock {
            tactics,
            span: Span::new(start, end),
        })
    }

    fn parse_tactic(&mut self) -> Option<Tactic> {
        let start = self.current_span().start;
        if self.check(TokenKind::Refl) {
            let span = self.current_span();
            self.advance();
            return Some(Tactic::Refl(span));
        }
        if self.eat(TokenKind::Exists) {
            let witness = self.parse_expr()?;
            let span = Span::new(start, witness.span().end);
            return Some(Tactic::Exists(witness, span));
        }
        let name = if self.check_ident() {
            self.current_text().to_string()
        } else {
            String::new()
        };
        match name.as_str() {
            "intro" => {
                self.advance();
                let binder = self.parse_ident()?;
                let span = Span::new(start, binder.span.end);
                Some(Tactic::Intro(binder, span))
            }
            "exact" | "apply" => {
                self.advance();
                let expr = self.parse_expr()?;
                let span = Span::new(start, expr.span().end);
                Some(if name == "exact" {
                    Tactic::Exact(expr, span)
                } else {
                    Tactic::Apply(expr, span)
                })
            }
            "rewrite" => {
                self.advance();
                let (rules, end) = self.parse_rewrite_rules(EXPECTED_A_RULE_LIST)?;
                Some(Tactic::Rewrite(rules, Span::new(start, end)))
            }
            "simp" => {
                self.advance();
                let (rules, end) = self.parse_rewrite_rules(EXPECTED_A_SIMP_LIST)?;
                Some(Tactic::Simp(rules, Span::new(start, end)))
            }
            "ring" => self
                .parse_tactic_alone(EXPECTED_RING_ALONE)
                .map(Tactic::Ring),
            "omega" => self
                .parse_tactic_alone(EXPECTED_OMEGA_ALONE)
                .map(Tactic::Omega),
            "left" => self
                .parse_tactic_alone(EXPECTED_LEFT_ALONE)
                .map(Tactic::Left),
            "right" => self
                .parse_tactic_alone(EXPECTED_RIGHT_ALONE)
                .map(Tactic::Right),
            "induction" => {
                self.advance();
                let target = self.parse_induction_target()?;
                let span = Span::new(start, target.span.end);
                Some(Tactic::Induction(target, span))
            }
            "cases" => {
                self.advance();
                let scrutinee = self.parse_expr()?;
                if self.check(TokenKind::LBrace)
                    && self.check_ahead_n(1, |kind| kind == TokenKind::LParen)
                {
                    return self.parse_cases_pair(start, scrutinee);
                }
                let (arms, end) = self.parse_cases_arms()?;
                Some(Tactic::Cases(
                    Box::new(scrutinee),
                    arms,
                    Span::new(start, end),
                ))
            }
            _ => {
                self.error(ParseErrorKind::Expected(EXPECTED_A_TACTIC.to_string()));
                None
            }
        }
    }

    /// A tactic that takes no arguments, its name current: its span, or
    /// `expected` when anything but the step's end follows it.
    fn parse_tactic_alone(&mut self, expected: &str) -> Option<Span> {
        let span = self.current_span();
        self.advance();
        if !self.check(TokenKind::Semi) && !self.check(TokenKind::RBrace) {
            self.error(ParseErrorKind::Expected(expected.to_string()));
            return None;
        }
        Some(span)
    }

    /// The one identifier `induction` takes, and nothing after it before the
    /// step ends: an expression target is Lean's `using` form (D1, §3).
    fn parse_induction_target(&mut self) -> Option<crate::ast::Ident> {
        if !self.check_ident() {
            self.error(ParseErrorKind::Expected(
                EXPECTED_AN_INDUCTION_TARGET.to_string(),
            ));
            return None;
        }
        let target = self.parse_ident()?;
        if !self.check(TokenKind::Semi) && !self.check(TokenKind::RBrace) {
            self.error(ParseErrorKind::Expected(
                EXPECTED_AN_INDUCTION_TARGET.to_string(),
            ));
            return None;
        }
        Some(target)
    }

    /// `{ arm, … }`, the `{` current; returns the arms and the `}`'s end.
    fn parse_cases_arms(&mut self) -> Option<(Vec<CasesArm>, u32)> {
        self.expect(TokenKind::LBrace)?;
        let mut arms = Vec::new();
        while !self.check(TokenKind::RBrace) {
            arms.push(self.parse_cases_arm()?);
            if !self.eat(TokenKind::Comma) {
                break;
            }
        }
        if arms.is_empty() {
            self.error(ParseErrorKind::Expected(EXPECTED_A_CASES_ARM.to_string()));
            return None;
        }
        let end = self.expect(TokenKind::RBrace)?.end;
        Some((arms, end))
    }

    /// `{ (k, e) => arm_body }`, the `{` current: the pair arm, one and only
    /// one (ADR 6.10.26e D6). `start` is the `cases` token's.
    fn parse_cases_pair(&mut self, start: u32, scrutinee: crate::ast::Expr) -> Option<Tactic> {
        self.expect(TokenKind::LBrace)?;
        self.expect(TokenKind::LParen)?;
        let witness = self.parse_cases_binder()?;
        if !self.eat(TokenKind::Comma) {
            self.error(ParseErrorKind::Expected(EXPECTED_A_PAIR_ARM.to_string()));
            return None;
        }
        let evidence = self.parse_cases_binder()?;
        if !self.eat(TokenKind::RParen) {
            self.error(ParseErrorKind::Expected(EXPECTED_A_PAIR_ARM.to_string()));
            return None;
        }
        self.expect(TokenKind::FatArrow)?;
        let (body, _) = self.parse_cases_arm_body()?;
        self.eat(TokenKind::Comma);
        let end = self.expect(TokenKind::RBrace)?.end;
        Some(Tactic::CasesPair(
            Box::new(scrutinee),
            witness,
            evidence,
            body,
            Span::new(start, end),
        ))
    }

    /// `C(b₁, …) => tactic` or `C => { tactic; … }`.
    fn parse_cases_arm(&mut self) -> Option<CasesArm> {
        let start = self.current_span().start;
        let ctor = self.parse_ident()?;
        let mut binders = Vec::new();
        if self.eat(TokenKind::LParen) {
            binders.push(self.parse_cases_binder()?);
            while self.eat(TokenKind::Comma) {
                binders.push(self.parse_cases_binder()?);
            }
            self.expect(TokenKind::RParen)?;
        }
        self.expect(TokenKind::FatArrow)?;
        let (body, end) = self.parse_cases_arm_body()?;
        Some(CasesArm {
            ctor,
            binders,
            body,
            span: Span::new(start, end),
        })
    }

    /// `tactic` or `{ tactic; … }`, after an arm's `=>`; returns the
    /// tactics and the body's end.
    fn parse_cases_arm_body(&mut self) -> Option<(Vec<Tactic>, u32)> {
        let (body, end) = if self.eat(TokenKind::LBrace) {
            let mut body = Vec::new();
            while !self.check(TokenKind::RBrace) {
                body.push(self.parse_tactic()?);
                if !self.eat(TokenKind::Semi) {
                    break;
                }
            }
            (body, self.expect(TokenKind::RBrace)?.end)
        } else {
            let tactic = self.parse_tactic()?;
            let end = tactic.span().end;
            (vec![tactic], end)
        };
        Some((body, end))
    }

    /// A name or `_`, followed by `,` or `)`: anything else is a nested
    /// pattern, which an arm does not take.
    #[allow(clippy::option_option)] // Reason: outer = parse failure, inner = `_`
    fn parse_cases_binder(&mut self) -> Option<Option<Ident>> {
        let binder = if self.eat(TokenKind::Underscore) {
            None
        } else if self.check_ident() {
            Some(self.parse_ident()?)
        } else {
            self.error(ParseErrorKind::Expected(
                EXPECTED_A_CASES_BINDER.to_string(),
            ));
            return None;
        };
        if !self.check(TokenKind::Comma) && !self.check(TokenKind::RParen) {
            self.error(ParseErrorKind::Expected(
                EXPECTED_A_CASES_BINDER.to_string(),
            ));
            return None;
        }
        Some(binder)
    }

    /// `[ rule, … ]`, the `[` current; returns the rules and the `]`'s end.
    /// `missing` is what a missing or empty list reports.
    fn parse_rewrite_rules(&mut self, missing: &str) -> Option<(Vec<RewriteRule>, u32)> {
        if !self.eat(TokenKind::LBracket) || self.check(TokenKind::RBracket) {
            self.error(ParseErrorKind::Expected(missing.to_string()));
            return None;
        }
        let mut rules = vec![self.parse_rewrite_rule()?];
        while self.eat(TokenKind::Comma) {
            rules.push(self.parse_rewrite_rule()?);
        }
        let end = self.expect(TokenKind::RBracket)?.end;
        Some((rules, end))
    }

    /// `<- e` or `e`.
    fn parse_rewrite_rule(&mut self) -> Option<RewriteRule> {
        let start = self.current_span().start;
        let reverse = self.eat(TokenKind::Lt);
        if reverse && !self.eat(TokenKind::Minus) {
            self.error(ParseErrorKind::Expected(
                EXPECTED_A_REVERSE_ARROW.to_string(),
            ));
            return None;
        }
        let rule = self.parse_expr()?;
        let span = Span::new(start, rule.span().end);
        Some(RewriteRule {
            reverse,
            rule,
            span,
        })
    }
}
