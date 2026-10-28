//! The two categories that are not elaboration errors (ADR 30.9.26j D8): the
//! parser's *Syntax* codes and `compile`'s *Build* codes. Grouped so
//! `explanations/` stays inside its file-count cap.

mod build;
mod syntax;

pub(super) use build::build;
pub(super) use syntax::syntax;
