//! `==` sugar: `expr == expr` in type position → `TypeExpr::Eq`.
//!
//! Three spellings are recognised anywhere a type is parsed: a literal start
//! (`0 == n`), `ident == expr` and `ident(args) == expr`. In **statement
//! position** — a function's return type, a theorem's or an axiom's
//! proposition — a bounded lookahead also admits a leading identifier or `(`
//! followed by operators (`n + 0 == n`), which the type parser otherwise reads
//! as a sum type (ADR 30.9.26d D10). `let`, lambda-parameter and `as`
//! annotations keep the three spellings only.

use crate::ast::*;
use crate::error::ParseErrorKind;
use crate::span::{Span, Spanned};
use crate::token::TokenKind;

use crate::parser::Parser;

/// Where a type is being parsed. Only a statement admits the lookahead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TypePosition {
    /// A return type, or a theorem's or axiom's proposition.
    Statement,
    /// Every other annotation: `let`, lambda parameters, `as`, type arguments.
    Annotation,
}

/// Whether the tokens from a statement's first token, up to its terminator,
/// hold an `==` at bracket depth 0 — i.e. whether the statement is `==` sugar
/// rather than a type. Only an identifier or `(` start is scanned; a literal
/// start is recognised without lookahead.
///
/// The terminators are the body's `{`, `;`, `=>`, `where`, end of input, and
/// the keywords that open the next item (an axiom's proposition has no body).
/// A `|` or an `as` inside an operand is scanned past, never a stop.
#[must_use]
pub(crate) fn annotation_is_eq_sugar(tokens: &[TokenKind]) -> bool {
    if !matches!(tokens.first(), Some(TokenKind::Ident | TokenKind::LParen)) {
        return false;
    }
    // `(n: Nat) -> n == n` is a dependent function type whose codomain holds
    // the `==`, never an equation (ADR 7.10.26n D1).
    if matches!(
        tokens,
        [TokenKind::LParen, TokenKind::Ident, TokenKind::Colon, ..]
    ) {
        return false;
    }
    let mut depth: usize = 0;
    for kind in tokens {
        match kind {
            TokenKind::LParen | TokenKind::LBracket => depth += 1,
            TokenKind::LBrace if depth > 0 => depth += 1,
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                let Some(outer) = depth.checked_sub(1) else {
                    return false;
                };
                depth = outer;
            }
            TokenKind::EqEq if depth == 0 => return true,
            kind if depth == 0 && ends_statement(*kind) => return false,
            _ => {}
        }
    }
    false
}

/// Whether one type argument, from its first token, is a term (ADR 6.10.26d
/// D2): it holds a literal, a `-`, or an identifier applied to `(`, and no
/// `==`. Each of those already fails to parse as a type, so no argument that
/// parsed before changes parse. The scan stops at a depth-0 `,` or `>`, and
/// answers "type" at any `<`, which only type arguments open: a term is
/// parsed at a precedence that never reads `<`.
#[must_use]
pub(crate) fn type_argument_is_term(tokens: impl IntoIterator<Item = TokenKind>) -> bool {
    let mut tokens = tokens.into_iter().peekable();
    let mut depth: usize = 0;
    let mut has_term_token = false;
    while let Some(kind) = tokens.next() {
        match kind {
            TokenKind::LParen | TokenKind::LBracket => depth += 1,
            TokenKind::RParen | TokenKind::RBracket => {
                let Some(outer) = depth.checked_sub(1) else {
                    break;
                };
                depth = outer;
            }
            TokenKind::Lt | TokenKind::EqEq => return false,
            TokenKind::Comma | TokenKind::Gt if depth == 0 => break,
            TokenKind::Eof => break,
            TokenKind::IntLiteral
            | TokenKind::True
            | TokenKind::False
            | TokenKind::StringLiteral
            | TokenKind::Minus => has_term_token = true,
            TokenKind::Ident if tokens.peek() == Some(&TokenKind::LParen) => {
                has_term_token = true;
            }
            _ => {}
        }
    }
    has_term_token
}

/// A token that ends a statement at bracket depth 0.
fn ends_statement(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::LBrace
            | TokenKind::Semi
            | TokenKind::FatArrow
            | TokenKind::Where
            | TokenKind::Eof
            | TokenKind::Fn
            | TokenKind::Pub
            | TokenKind::Theorem
            | TokenKind::Axiom
            | TokenKind::Type
            | TokenKind::Struct
            | TokenKind::Enum
            | TokenKind::Mod
            | TokenKind::Use
            | TokenKind::Extern
            | TokenKind::Impl
            | TokenKind::Trait
            | TokenKind::Const
            | TokenKind::Hash
    )
}

impl Parser<'_> {
    /// Parse a type at `position`; a statement whose lookahead finds `==`
    /// parses as `==` sugar, everything else exactly as [`Parser::parse_type`].
    pub(in crate::parser) fn parse_type_in(&mut self, position: TypePosition) -> Option<TypeExpr> {
        if position == TypePosition::Statement && self.statement_is_eq_sugar() {
            // An operand's trailing path must not open a named record: in
            // `-> x == Some { refl }` the `{` is the body (ADR 30.9.26d D10).
            let saved = std::mem::replace(&mut self.named_records_allowed, false);
            let sugar = self.parse_eq_sugar();
            self.named_records_allowed = saved;
            return sugar;
        }
        self.parse_type()
    }

    fn statement_is_eq_sugar(&self) -> bool {
        let kinds: Vec<TokenKind> = self.tokens[self.pos.min(self.tokens.len())..]
            .iter()
            .map(|token| token.kind)
            .collect();
        annotation_is_eq_sugar(&kinds)
    }

    /// One type argument: a term when [`type_argument_is_term`] says its
    /// tokens cannot be a type, else a type exactly as before (ADR 6.10.26d).
    pub(super) fn parse_type_arg(&mut self) -> Option<TypeExpr> {
        let rest = &self.tokens[self.pos.min(self.tokens.len())..];
        if type_argument_is_term(rest.iter().map(|token| token.kind)) {
            let term = self.parse_expr_prec(5)?; // stops before `<`, `>` and `==`
            return Some(TypeExpr::Term(Box::new(term)));
        }
        self.parse_type()
    }

    /// Check if the current token can only start an expression, not a type.
    /// If so, we must be in `expr == expr` equality sugar mode.
    pub(super) fn is_eq_sugar_start(&self) -> bool {
        matches!(
            self.current().kind,
            TokenKind::IntLiteral | TokenKind::True | TokenKind::False | TokenKind::StringLiteral
        )
    }

    /// Parse `expr == expr` when the LHS is known to be expression-only.
    pub(super) fn parse_eq_sugar(&mut self) -> Option<TypeExpr> {
        let lhs = self.parse_expr_prec(4)?; // stop before ==
        self.expect(TokenKind::EqEq)?;
        let rhs = self.parse_expr_prec(4)?;
        let span = Span::new(lhs.span().start, rhs.span().end);
        Some(TypeExpr::Eq(Box::new(lhs), Box::new(rhs), span))
    }

    /// After parsing a type that turns out to be followed by `==`,
    /// convert the type LHS to an expression and parse the RHS.
    /// Only path types (identifiers) can be reinterpreted as expressions.
    pub(super) fn parse_eq_sugar_after_type(&mut self, lhs_ty: TypeExpr) -> Option<TypeExpr> {
        let start = lhs_ty.span().start;
        match lhs_ty {
            TypeExpr::Path(path) => {
                self.expect(TokenKind::EqEq)?;
                let rhs = self.parse_expr_prec(4)?;
                let span = Span::new(start, rhs.span().end);
                Some(TypeExpr::Eq(
                    Box::new(Expr::Path(path)),
                    Box::new(rhs),
                    span,
                ))
            }
            _ => {
                self.error(ParseErrorKind::Expected(
                    "identifier or literal on left side of `==` in type position".to_string(),
                ));
                None
            }
        }
    }

    /// Parse `ident(args...) == expr` in type position.
    /// Called when a path was parsed as a type, then `(` follows.
    pub(super) fn parse_eq_sugar_fn_call(&mut self, path: Path) -> Option<TypeExpr> {
        let start = path.span.start;
        let mut lhs: Expr = Expr::Path(path);
        // Parse postfix operations (calls, field access) using the expr parser
        lhs = self.parse_postfix_from(lhs)?;
        self.expect(TokenKind::EqEq)?;
        let rhs = self.parse_expr_prec(4)?;
        let span = Span::new(start, rhs.span().end);
        Some(TypeExpr::Eq(Box::new(lhs), Box::new(rhs), span))
    }

    /// Parse postfix operations (calls, field access) on an already-parsed expression.
    fn parse_postfix_from(&mut self, mut expr: Expr) -> Option<Expr> {
        while let TokenKind::LParen = self.current().kind {
            let args = self.parse_call_args()?;
            let span = Span::new(expr.span().start, self.prev_span().end);
            expr = Expr::App(Box::new(expr), args, span);
        }
        Some(expr)
    }
}
