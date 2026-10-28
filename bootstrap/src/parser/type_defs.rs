//! Parsing type definitions: `type`, `struct`, `enum`.

use crate::ast::*;
use crate::error::ParseErrorKind;
use crate::span::{Span, Spanned};
use crate::token::TokenKind;

use super::Parser;

impl Parser<'_> {
    pub(super) fn parse_type_item(&mut self) -> Option<Item> {
        let start = self.current_span().start;

        // Parse visibility
        let visibility = self.parse_visibility();

        self.expect(TokenKind::Type)?;

        let name = self.parse_ident()?;
        let type_params = self.parse_optional_type_params();
        let (value_params, sort) = self.parse_family_header()?;

        if self.eat(TokenKind::Eq) {
            // Could be type alias, ADT, or record type
            if sort.is_some() {
                // A sorted definition is always an alias, never variants (ADR 4.10.26i D5).
                let ty = self.parse_type()?;
                let end = ty.span().end;
                Some(Item::TypeAlias(TypeAlias {
                    visibility,
                    name,
                    type_params,
                    value_params,
                    sort,
                    ty,
                    span: Span::new(start, end),
                }))
            } else if self.check(TokenKind::LBrace) {
                // Record type: type Point = { x: Nat, y: Nat }
                let (record_fields, end) = self.parse_record_type_body()?;
                Some(Item::TypeDef(TypeDef {
                    visibility,
                    name,
                    type_params,
                    body: TypeBody::Record(record_fields),
                    span: Span::new(start, end),
                }))
            } else if self.check(TokenKind::Pipe)
                || self.check(TokenKind::Pub)
                || (self.check_ident()
                    && (self.check_ahead(TokenKind::LParen) || self.check_ahead(TokenKind::Pipe)))
            {
                // ADT with variants:
                //   | Foo(X)        — pipe-led
                //   pub Foo(X)      — visibility on variant
                //   Foo(X) | Bar(Y) — ident followed by ( or |
                // Ident followed by < is a type alias (generic application).
                // Bare ident (no lookahead match) falls through to type alias.
                let variants = self.parse_variants()?;
                let end = variants.last().map_or(name.span.end, |v| v.span.end);
                Some(Item::TypeDef(TypeDef {
                    visibility,
                    name,
                    type_params,
                    body: TypeBody::Sum(variants),
                    span: Span::new(start, end),
                }))
            } else {
                // Type alias: bare ident, generic application, or other type expression
                let ty = self.parse_type()?;
                let end = ty.span().end;
                Some(Item::TypeAlias(TypeAlias {
                    visibility,
                    name,
                    type_params,
                    value_params,
                    sort,
                    ty,
                    span: Span::new(start, end),
                }))
            }
        } else if self.check(TokenKind::LBrace) {
            // ADT with braces (Rust-like enum syntax)
            self.expect(TokenKind::LBrace)?;
            let variants = self.parse_variants()?;
            let end = self.expect(TokenKind::RBrace)?.end;
            Some(Item::TypeDef(TypeDef {
                visibility,
                name,
                type_params,
                body: TypeBody::Sum(variants),
                span: Span::new(start, end),
            }))
        } else {
            self.error(ParseErrorKind::Expected(
                "`=` or block after type name".to_string(),
            ));
            None
        }
    }

    /// The optional `(x: A, …)` and `: Sort` between a type's name and its `=`
    /// (ADR 4.10.26i D5). A parameter list requires the sort annotation.
    fn parse_family_header(&mut self) -> Option<(Vec<Param>, Option<SortAnnot>)> {
        let has_params = self.check(TokenKind::LParen);
        let value_params = if has_params {
            self.parse_params()?
        } else {
            Vec::new()
        };
        let sort = if self.eat(TokenKind::Colon) {
            Some(self.parse_sort_annot()?)
        } else {
            None
        };
        if has_params && sort.is_none() {
            self.error(ParseErrorKind::Expected(
                "`: Prop` or `: Type` after a type's parameter list".to_string(),
            ));
            return None;
        }
        Some((value_params, sort))
    }

    /// `Prop` (a keyword) or `Type` (an identifier) in a sort position.
    fn parse_sort_annot(&mut self) -> Option<SortAnnot> {
        if self.eat(TokenKind::Prop) {
            return Some(SortAnnot::Prop);
        }
        if self.check(TokenKind::Ident) && self.current_text() == "Type" {
            self.advance();
            return Some(SortAnnot::Type);
        }
        self.error(ParseErrorKind::Expected(
            "`Prop` or `Type` after `:`".to_string(),
        ));
        None
    }

    /// Parse a record type body: `{ field: Type, ... }`
    pub(super) fn parse_record_type_body(&mut self) -> Option<(Vec<RecordField>, u32)> {
        let _start = self.expect(TokenKind::LBrace)?;

        let mut fields = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.at_eof() {
            let field_start = self.current_span().start;

            // Parse optional visibility modifier on the field
            let visibility = if self.check(TokenKind::Pub) {
                Some(self.parse_visibility())
            } else {
                None
            };

            let name = self.parse_ident()?;
            self.expect(TokenKind::Colon)?;
            let ty = self.parse_type()?;
            let field_end = ty.span().end;

            fields.push(RecordField {
                visibility,
                name,
                ty,
                span: Span::new(field_start, field_end),
            });

            if !self.eat(TokenKind::Comma) {
                break;
            }
        }

        let end = self.expect(TokenKind::RBrace)?.end;
        Some((fields, end))
    }

    pub(super) fn parse_struct(&mut self) -> Option<TypeDef> {
        let start = self.current_span().start;

        // Parse visibility
        let visibility = self.parse_visibility();

        self.expect(TokenKind::Struct)?;

        let name = self.parse_ident()?;
        let type_params = self.parse_optional_type_params();

        self.expect(TokenKind::LBrace)?;
        let fields = self.parse_struct_fields()?;
        let end = self.expect(TokenKind::RBrace)?.end;

        // Convert struct to single-variant type def
        let variant = Variant {
            visibility: None,
            name: name.clone(),
            fields,
            span: Span::new(start, end),
        };

        Some(TypeDef {
            visibility,
            name,
            type_params,
            body: TypeBody::Sum(vec![variant]),
            span: Span::new(start, end),
        })
    }

    pub(super) fn parse_enum(&mut self) -> Option<TypeDef> {
        let start = self.current_span().start;

        // Parse visibility
        let visibility = self.parse_visibility();

        self.expect(TokenKind::Enum)?;

        let name = self.parse_ident()?;
        let type_params = self.parse_optional_type_params();

        self.expect(TokenKind::LBrace)?;
        let variants = self.parse_enum_variants()?;
        let end = self.expect(TokenKind::RBrace)?.end;

        Some(TypeDef {
            visibility,
            name,
            type_params,
            body: TypeBody::Sum(variants),
            span: Span::new(start, end),
        })
    }
}
