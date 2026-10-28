//! Parsing type expressions and type definitions (variants, structs, enums).

use crate::ast::*;
use crate::error::{ParseErrorKind, Suggestion};
use crate::span::{Span, Spanned};
use crate::token::TokenKind;

mod eq_sugar;
mod variants;

pub(crate) use eq_sugar::{annotation_is_eq_sugar, type_argument_is_term, TypePosition};

use super::Parser;

impl Parser<'_> {
    // ─────────────────────────────────────────────────────────────────────────
    // Type Expressions
    // ─────────────────────────────────────────────────────────────────────────

    pub(super) fn parse_type(&mut self) -> Option<TypeExpr> {
        // Check for == sugar: if current token is expression-only (int, bool literal),
        // this must be `expr == expr` equality sugar in type position.
        if self.is_eq_sugar_start() {
            return self.parse_eq_sugar();
        }

        let mut ty = self.parse_type_prec(0)?;

        // After parsing a type, check for == sugar: `path == expr`
        // where the LHS type is reinterpreted as an expression.
        if self.check(TokenKind::EqEq) {
            ty = self.parse_eq_sugar_after_type(ty)?;
        } else if self.check(TokenKind::LParen) {
            // `ident(args...) == expr` — function call in == sugar.
            // In type position, `Path(` is never valid, so this must be
            // an expression (function call) followed by `==`.
            if let TypeExpr::Path(path) = ty {
                return self.parse_eq_sugar_fn_call(path);
            }
        }

        Some(ty)
    }

    fn parse_type_prec(&mut self, min_prec: u8) -> Option<TypeExpr> {
        let mut left = self.parse_type_atom()?;

        loop {
            // Arrow type: A -> B (right associative, precedence 1)
            if min_prec <= 1 && self.check(TokenKind::Arrow) {
                self.advance();
                let right = self.parse_type_prec(1)?;
                let span = Span::new(left.span().start, right.span().end);
                left = TypeExpr::Arrow(Box::new(left), Box::new(right), span);
                continue;
            }

            // Product type: A * B (precedence 3)
            if min_prec <= 3 && self.check(TokenKind::Star) {
                self.advance();
                let right = self.parse_type_prec(4)?;
                let span = Span::new(left.span().start, right.span().end);
                left = TypeExpr::Product(Box::new(left), Box::new(right), span);
                continue;
            }

            // Sum type: A + B (precedence 2)
            if min_prec <= 2 && self.check(TokenKind::Plus) {
                self.advance();
                let right = self.parse_type_prec(3)?;
                let span = Span::new(left.span().start, right.span().end);
                left = TypeExpr::Sum(Box::new(left), Box::new(right), span);
                continue;
            }

            break;
        }

        Some(left)
    }

    fn parse_type_atom(&mut self) -> Option<TypeExpr> {
        let start = self.current_span().start;

        match self.current().kind {
            TokenKind::Bool | TokenKind::Nat => {
                let name = if self.current().kind == TokenKind::Bool {
                    "Bool"
                } else {
                    "Nat"
                };
                self.advance();
                Some(TypeExpr::Path(Path::simple(Ident::new(
                    name,
                    Span::new(start, self.prev_span().end),
                ))))
            }
            TokenKind::Unit => {
                self.advance();
                Some(TypeExpr::Unit(Span::new(start, self.prev_span().end)))
            }
            TokenKind::Void | TokenKind::Bang => {
                self.advance();
                Some(TypeExpr::Void(Span::new(start, self.prev_span().end)))
            }
            TokenKind::Prop => {
                self.advance();
                Some(TypeExpr::Prop(Span::new(start, self.prev_span().end)))
            }
            TokenKind::Forall => {
                self.advance();
                let name = self.parse_ident()?;
                if self.check(TokenKind::Colon) {
                    // `forall` binds a TYPE variable; a value binder has its
                    // own spelling (ADR 7.10.26n D1).
                    self.error_expected(TokenKind::Dot);
                    self.add_help_to_last_error(
                        Span::new(start, start),
                        "a value binder is written `(n: Nat) -> …`",
                    );
                    return None;
                }
                self.expect(TokenKind::Dot)?;
                let body = self.parse_type()?;
                let end = body.span().end;
                Some(TypeExpr::Forall(
                    name,
                    Box::new(body),
                    Span::new(start, end),
                ))
            }
            // `exists x: A. B` (ADR 4.10.26h D5): the body is a statement, so
            // `n + k == m` reads as `==` sugar inside it, wherever it appears.
            TokenKind::Exists => {
                self.advance();
                let name = self.parse_ident()?;
                self.expect(TokenKind::Colon)?;
                let binder_ty = self.parse_type()?;
                self.expect(TokenKind::Dot)?;
                let body = self.parse_type_in(TypePosition::Statement)?;
                let end = body.span().end;
                Some(TypeExpr::Exists(
                    name,
                    Box::new(binder_ty),
                    Box::new(body),
                    Span::new(start, end),
                ))
            }
            TokenKind::Star => {
                self.advance();
                let inner = self.parse_type_atom()?;
                let end = inner.span().end;
                Some(TypeExpr::Ptr(Box::new(inner), Span::new(start, end)))
            }
            TokenKind::Ref => self.parse_ref_type(start),
            TokenKind::LParen => self.parse_paren_or_tuple_type(start),
            TokenKind::Ident => self.parse_ident_type(start),
            _ => {
                self.error(ParseErrorKind::InvalidType);
                Some(TypeExpr::Error(self.current_span()))
            }
        }
    }

    /// Parse a `Ref<T>` type or bare `Ref` identifier.
    fn parse_ref_type(&mut self, start: u32) -> Option<TypeExpr> {
        self.advance();
        if self.check(TokenKind::Lt) {
            self.parse_single_type_arg_as(start, "Ref", |inner, span| {
                TypeExpr::Ref(Box::new(inner), span)
            })
        } else {
            Some(TypeExpr::Path(Path::simple(Ident::new(
                "Ref",
                Span::new(start, self.prev_span().end),
            ))))
        }
    }

    /// Parse a parenthesized type `(T)`, unit `()`, or tuple type `(A, B, C)`.
    fn parse_paren_or_tuple_type(&mut self, start: u32) -> Option<TypeExpr> {
        self.advance();
        if self.check(TokenKind::RParen) {
            self.advance();
            return Some(TypeExpr::Unit(Span::new(start, self.prev_span().end)));
        }

        // `( Ident :` opens a dependent binder: inside a type's parentheses an
        // identifier is never otherwise followed by a single `:` (ADR 7.10.26n D1).
        if self.check(TokenKind::Ident) && self.check_ahead(TokenKind::Colon) {
            return self.parse_dependent_function_type(start);
        }

        let first = self.parse_type()?;
        if self.check(TokenKind::Comma) {
            // Tuple type: (A, B, C) -> A * B * C
            let mut types = vec![first];
            while self.eat(TokenKind::Comma) {
                if self.check(TokenKind::RParen) {
                    break;
                }
                types.push(self.parse_type()?);
            }
            self.expect(TokenKind::RParen)?;
            // Fold into right-associative nested Product: A * (B * C)
            let result = types
                .into_iter()
                .rev()
                .reduce(|acc, ty| {
                    let span = Span::new(ty.span().start, acc.span().end);
                    TypeExpr::Product(Box::new(ty), Box::new(acc), span)
                })
                .unwrap();
            Some(result)
        } else {
            let end = self.expect(TokenKind::RParen)?.end;
            Some(TypeExpr::Paren(Box::new(first), Span::new(start, end)))
        }
    }

    /// Parse `(x: A) -> B` after its `(` (ADR 7.10.26n D1): one binder per pair
    /// of parentheses, and the arrow is required. The codomain is a statement,
    /// as `exists`' body is, so `(n: Nat) -> n == n` reads `n == n` as sugar;
    /// it extends to the end of the type, so the arrow stays right-associative.
    fn parse_dependent_function_type(&mut self, start: u32) -> Option<TypeExpr> {
        let binder = self.parse_ident()?;
        self.expect(TokenKind::Colon)?;
        let domain = self.parse_type()?;
        if self.check(TokenKind::Comma) {
            self.error_expected(TokenKind::RParen);
            let comma = self.current_span();
            self.add_help_to_last_error(
                Span::new(comma.start, comma.start),
                "a dependent function type takes one binder per pair of parentheses: `(x: A) -> (y: B) -> C`",
            );
            return None;
        }
        self.expect(TokenKind::RParen)?;
        if !self.check(TokenKind::Arrow) {
            self.error_expected(TokenKind::Arrow);
            if let Some(error) = self.errors.last_mut() {
                error.expected = vec!["`->` after a dependent binder".to_string()];
            }
            return None;
        }
        self.advance();
        let codomain = self.parse_type_in(TypePosition::Statement)?;
        let end = codomain.span().end;
        Some(TypeExpr::Pi(
            binder,
            Box::new(domain),
            Box::new(codomain),
            Span::new(start, end),
        ))
    }

    /// Attach a `help:` line to the error just pushed.
    fn add_help_to_last_error(&mut self, at: Span, message: &str) {
        if let Some(error) = self.errors.last_mut() {
            error.suggestions.push(Suggestion::new(at, "", message));
        }
    }

    /// Parse an identifier-led type: plain path, `Ref<T>`, `Eq<T, a, b>`, or generic `Name<Args>`.
    fn parse_ident_type(&mut self, start: u32) -> Option<TypeExpr> {
        let path = self.parse_path()?;
        if path.is_simple() && path.item_name().name == "Ref" && self.check(TokenKind::Lt) {
            self.parse_single_type_arg_as(start, "Ref", |inner, span| {
                TypeExpr::Ref(Box::new(inner), span)
            })
        } else if path.is_simple() && path.item_name().name == "Eq" && self.check(TokenKind::Lt) {
            self.parse_eq_type_args(start)
        } else if self.check(TokenKind::Lt) {
            let args = self.parse_type_args()?;
            let end = self.prev_span().end;
            Some(TypeExpr::App(
                Box::new(TypeExpr::Path(path)),
                args,
                Span::new(start, end),
            ))
        } else {
            Some(TypeExpr::Path(path))
        }
    }

    /// Parse `Eq<T, a, b>` — one type argument and two expression arguments.
    /// Uses `parse_expr_prec(5)` to stop before `<`/`>` comparison operators.
    fn parse_eq_type_args(&mut self, start: u32) -> Option<TypeExpr> {
        self.expect(TokenKind::Lt)?;
        let ty_arg = self.parse_type()?;
        self.expect(TokenKind::Comma)?;
        let lhs = self.parse_expr_prec(5)?;
        self.expect(TokenKind::Comma)?;
        let rhs = self.parse_expr_prec(5)?;
        self.expect(TokenKind::Gt)?;
        let end = self.prev_span().end;
        Some(TypeExpr::EqExplicit(
            Box::new(ty_arg),
            Box::new(lhs),
            Box::new(rhs),
            Span::new(start, end),
        ))
    }

    /// Parse `<T>` expecting exactly one type argument, then apply the constructor.
    fn parse_single_type_arg_as(
        &mut self,
        start: u32,
        type_name: &str,
        ctor: impl FnOnce(TypeExpr, Span) -> TypeExpr,
    ) -> Option<TypeExpr> {
        let args = self.parse_type_args()?;
        if args.len() != 1 {
            self.error(ParseErrorKind::Expected(format!(
                "exactly one type argument for {}",
                type_name
            )));
            return Some(TypeExpr::Error(Span::new(start, self.prev_span().end)));
        }
        let end = self.prev_span().end;
        Some(ctor(
            args.into_iter().next().unwrap(),
            Span::new(start, end),
        ))
    }

    pub(super) fn parse_type_args(&mut self) -> Option<Vec<TypeExpr>> {
        self.expect(TokenKind::Lt)?;

        let mut args = Vec::new();
        loop {
            if self.check(TokenKind::Gt) || self.at_eof() {
                break;
            }

            args.push(self.parse_type_arg()?);

            if !self.eat(TokenKind::Comma) {
                break;
            }
        }

        self.expect(TokenKind::Gt)?;
        Some(args)
    }
}
