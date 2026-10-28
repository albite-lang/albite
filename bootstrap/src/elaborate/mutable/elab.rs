//! Where the elaborator meets `B4.2` (ADR 13.10.26r D5): the scope a block's
//! lowering reads from the environment, the region and closure depths each
//! local records, and E0105 at variable resolution.

use crate::ast::{Expr, Stmt};
use crate::elaborate::error::{ElabError, ElabErrorKind, MutableErrorKind};
use crate::elaborate::{ElabResult, Elaborator};
use crate::span::Span;
use albite_core::{Term, Type};

use super::{
    lower_joins, needs_lowering, BindingFacts, JoinSource, LoweredBlock, OuterScope, ValueHint,
};

impl Elaborator<'_> {
    /// Lower a block's assignments and joins before it is elaborated, with
    /// the scope outside it read from the environment; `None` when the block
    /// holds no assignment, which is every block written before `B4.2`.
    pub(crate) fn lower_block_joins(
        &self,
        stmts: &[Stmt],
        tail: Option<&Expr>,
        span: Span,
    ) -> ElabResult<Option<LoweredBlock>> {
        if !needs_lowering(stmts, tail) {
            return Ok(None);
        }
        let env = &self.env;
        let lookup = |name: &str| match env.lookup_local(name) {
            Some(binding) => Some(BindingFacts {
                mutable: binding.mutable,
                region_depth: binding.depths.region,
                closure_depth: binding.depths.closure,
            }),
            // A global is never `mut`.
            None => env.lookup_value(name).map(|_| BindingFacts {
                mutable: false,
                region_depth: 0,
                closure_depth: 0,
            }),
        };
        let scope = OuterScope {
            lookup: &lookup,
            region_depth: env.mutable_depths.region,
            closure_depth: env.mutable_depths.closure,
        };
        lower_joins(stmts, tail, &scope, span.start)
            .map(Some)
            .map_err(|refusal| mutable_error(refusal.span, refusal.kind))
    }

    /// Elaborate a `let`'s value: checked against the joined types when the
    /// binders are a join's and every type is known (D3, "the join is
    /// typed"), else inferred as before.
    pub(crate) fn check_or_infer_join(
        &mut self,
        names: &[&str],
        value: &Expr,
    ) -> ElabResult<(Term, Type)> {
        match self.join_type(names) {
            Some(ty) => Ok((self.check(value, &ty)?, ty)),
            None => self.infer(value),
        }
    }

    /// The type of the tuple a join unpacks into `names`, or `None` when they
    /// are not a join's or one of the types is not known.
    fn join_type(&mut self, names: &[&str]) -> Option<Type> {
        if names.is_empty() {
            return None;
        }
        let mut types = Vec::with_capacity(names.len());
        for name in names {
            let (joins, expected) = self
                .join_frames
                .iter()
                .rev()
                .find(|(joins, _)| joins.contains_key(*name))?;
            let ty = match joins.get(*name)? {
                JoinSource::Binding(source) => self.env.lookup_local(source)?.ty.clone(),
                JoinSource::Value(ValueHint::BlockExpected) => expected.clone()?,
                JoinSource::Value(ValueHint::TypeOf(target)) => {
                    self.env.lookup_local(target)?.ty.clone()
                }
                JoinSource::Value(ValueHint::Annotation(annotation)) => {
                    let annotation = annotation.clone();
                    self.elab_type(&annotation).ok()?
                }
                JoinSource::Value(ValueHint::Unknown) => return None,
            };
            types.push(ty);
        }
        let last = types.pop()?;
        Some(types.into_iter().rev().fold(last, |rest, item| {
            Type::Product(Box::new(item), Box::new(rest))
        }))
    }

    /// E0105 when `name` is a `mut` local declared outside the lambda being
    /// elaborated (M3: any mention, not only an escaping one).
    pub(crate) fn check_not_captured_mutable(&self, name: &str, span: Span) -> ElabResult<()> {
        let captured = self.env.lookup_local(name).is_some_and(|binding| {
            binding.mutable && binding.depths.closure < self.env.mutable_depths.closure
        });
        if captured {
            let kind = MutableErrorKind::Captured {
                name: name.to_string(),
            };
            return Err(mutable_error(span, kind));
        }
        Ok(())
    }

    /// Run `body` one lambda deeper: a `mut` local bound outside is off limits.
    pub(crate) fn in_closure<T>(&mut self, body: impl FnOnce(&mut Self) -> T) -> T {
        self.env.mutable_depths.closure += 1;
        let result = body(self);
        self.env.mutable_depths.closure -= 1;
        result
    }

    /// Run `body` one region deeper: a `mut` local bound outside may be read
    /// but not assigned (M6).
    pub(crate) fn in_region<T>(&mut self, body: impl FnOnce(&mut Self) -> T) -> T {
        self.env.mutable_depths.region += 1;
        let result = body(self);
        self.env.mutable_depths.region -= 1;
        result
    }
}

pub(crate) fn mutable_error(span: Span, kind: MutableErrorKind) -> ElabError {
    ElabError::new(span, ElabErrorKind::Mutable(kind))
}
