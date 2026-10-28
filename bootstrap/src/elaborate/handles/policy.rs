//! What the use walk is configured by and reports (ADR 7.10.26t D4, D5), and
//! the two syntactic readings it makes of a call — split from `uses.rs` to keep
//! that file under its size threshold.

use crate::ast::{Expr, Path};
use crate::span::Span;

use super::HandleDeclarations;

/// How the arms of one `if` or `match` combine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmPolicy {
    /// The arms are alternatives: a handle consumed in each is consumed once.
    Max,
    /// The arms add up, as `BufferReused` counts them.
    Sum,
}

/// The two choices of the counting walk (D5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsePolicy {
    /// Whether an observer before the first consuming use is free.
    pub observers_free: bool,
    pub arms: ArmPolicy,
}

impl UsePolicy {
    /// `B4.1`'s rule (M2).
    pub const AFFINE: UsePolicy = UsePolicy {
        observers_free: true,
        arms: ArmPolicy::Max,
    };
}

/// Which refusal a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleFindingKind {
    UsedAfterConsumed,
    CapturedByClosure,
    /// A region body reaches a handle (ADR 9.10.26o D4, E0095).
    ReachedFromRegion,
}

/// One refusal: the handle, and the use that broke the rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandleFinding {
    pub kind: HandleFindingKind,
    pub handle: String,
    pub span: Span,
}

/// The name a single-segment path is; `None` for a qualified one.
pub(super) fn single_segment(path: &Path) -> Option<&str> {
    let [segment] = path.segments.as_slice() else {
        return None;
    };
    Some(segment.name.as_str())
}

/// The bare name a call's callee is, through parentheses and type arguments.
pub(super) fn callee_name(callee: &Expr) -> Option<&str> {
    match callee {
        Expr::Paren(inner, _) | Expr::TypeApp(inner, _, _) => callee_name(inner),
        Expr::Path(path) => path.segments.last().map(|segment| segment.name.as_str()),
        _ => None,
    }
}

/// Whether a `let`'s definition yields a handle (D4): a handle variable, a
/// call whose callee's declared result contains a handle, or an `if`, `match`
/// or block one of whose results does.
pub(super) fn yields_handle(
    definition: &Expr,
    is_handle_variable: &dyn Fn(&str) -> bool,
    declarations: &HandleDeclarations,
) -> bool {
    let yields = |expr: &Expr| yields_handle(expr, is_handle_variable, declarations);
    match definition {
        Expr::Paren(inner, _) | Expr::Annot(inner, _, _) | Expr::Try(inner, _) => yields(inner),
        Expr::Path(path) => single_segment(path).is_some_and(is_handle_variable),
        Expr::App(callee, _, _) => {
            callee_name(callee).is_some_and(|name| declarations.returns_handle(name))
        }
        Expr::If(_, then, otherwise, _) => yields(then) || yields(otherwise),
        Expr::Match(_, arms, _) => arms.iter().any(|arm| yields(&arm.body)),
        Expr::Block(_, Some(tail), _) => yields(tail),
        _ => false,
    }
}

/// An argument that is a bare variable, through parentheses.
pub(super) fn bare_variable(arg: &Expr) -> Option<(&str, Span)> {
    match arg {
        Expr::Paren(inner, _) => bare_variable(inner),
        Expr::Path(path) => single_segment(path).map(|name| (name, path.span)),
        _ => None,
    }
}
