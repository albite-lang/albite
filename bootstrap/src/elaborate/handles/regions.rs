//! E0095's handle half (ADR 9.10.26o D4): inside a `region { … }` body, the
//! use walk refuses every route a resource handle could take in — a handle
//! variable bound outside the body, a function whose declared type contains
//! a handle, and a handle constructor.
//!
//! Elaboration erases the handle wrapper (7.10.26t D2), so this reads
//! declarations at admission, as `B4.1`'s rule does, and rides the same walk:
//! a region body is a floor below which every handle id was bound outside it,
//! the way a closure body is.

use crate::ast::{Expr, Path};

use super::UseWalk;
use crate::elaborate::handles::policy::{single_segment, HandleFindingKind};
use crate::elaborate::handles::HANDLE_TYPES;

impl UseWalk<'_> {
    /// Walk a region body: every handle bound before it is outside it.
    pub(super) fn region(&mut self, body: &Expr) {
        let floor = self.region_floor.replace(self.next_id);
        self.expr(body);
        self.region_floor = floor;
    }

    /// Inside a region, refuse a path that names an outer handle variable, a
    /// function whose declared type contains a handle, or a handle constructor.
    pub(super) fn region_reference(&mut self, path: &Path) {
        let Some(floor) = self.region_floor else {
            return;
        };
        let Some(name) = path.segments.last().map(|segment| segment.name.as_str()) else {
            return;
        };
        let reached = match single_segment(path) {
            Some(local) if self.is_bound(local) => {
                let id = self.handle_id(local);
                id != 0 && id < floor
            }
            _ => self.declarations.mentions_handle(name) || is_handle_constructor(name),
        };
        if reached {
            self.report(HandleFindingKind::ReachedFromRegion, name, path.span);
        }
    }

    fn is_bound(&self, name: &str) -> bool {
        self.scope.iter().any(|(bound, _)| bound == name)
    }
}

/// Whether `name` is a listed handle's constructor (`MkBytes`, `MkStringBuilder`).
fn is_handle_constructor(name: &str) -> bool {
    HANDLE_TYPES
        .iter()
        .any(|(_, constructor)| *constructor == name)
}
