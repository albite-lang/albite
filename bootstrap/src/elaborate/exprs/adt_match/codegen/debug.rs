//! Debug helpers for ADT match codegen.
//!
//! Provides debug tracing for match elaboration and type unfolding.

use std::collections::HashMap;

use crate::ast;
use albite_core::Term;

use crate::elaborate::env as elab_env;
use crate::elaborate::Elaborator;

/// Check if debug tracing is enabled for type unfolding.
/// Replaced by --trace-types (ADR 13.4.26c §5), kept for match codegen fallback.
pub(super) fn debug_unfold_enabled() -> bool {
    albite_runtime::settings::read("DEBUG_UNFOLD").is_some_and(|v| v == "1")
}

/// Check if debug tracing is enabled for match codegen.
/// Set ALBITE_DEBUG_MATCH=1 to enable.
pub(super) fn debug_match_enabled() -> bool {
    albite_runtime::settings::read("DEBUG_MATCH").is_some_and(|v| v == "1")
}

impl<'a> Elaborator<'a> {
    /// Debug output at the start of ADT match elaboration.
    pub(super) fn debug_adt_match_start(
        &self,
        ctor_arms: &HashMap<usize, Vec<&ast::MatchArm>>,
        catch_all_arm: Option<&ast::MatchArm>,
        constructors: &[elab_env::Constructor],
        ctor_index: usize,
        adt_name: &str,
    ) {
        if debug_match_enabled() && ctor_index == 0 {
            eprintln!("\n=== ADT Match Debug for {} ===", adt_name);
            eprintln!("ctor_arms keys: {:?}", ctor_arms.keys().collect::<Vec<_>>());
            eprintln!("catch_all_arm: {:?}", catch_all_arm.is_some());
            for (idx, ctor) in constructors.iter().enumerate() {
                let arm_count = ctor_arms.get(&idx).map(|v| v.len()).unwrap_or(0);
                eprintln!("  [{}] {} -> {} arm(s)", idx, ctor.name, arm_count);
            }
        }
    }

    /// Debug output for specific constructor bodies.
    pub(super) fn debug_ctor_body(
        &self,
        left_var: &str,
        left_body: &Term,
        ctor_index: usize,
        adt_name: &str,
    ) {
        if debug_match_enabled() && adt_name == "TokenKind" && ctor_index == 83 {
            eprintln!(
                "[TokenKind TokEof] left_var={}, body={:?}",
                left_var, left_body
            );
        }
    }
}
