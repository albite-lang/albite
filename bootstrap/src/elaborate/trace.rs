//! Tracing utilities for the elaborator (--trace-types support).
//!
//! Provides targeted tracing of type elaboration for a specific definition,
//! including semantic provenance annotations from ADR 13.4.26c §5.

use albite_core::Type;

use super::Elaborator;

/// Seed `trace_encoding` from the `ALBITE_TRACE_ENCODING` setting (ADR
/// 22.7.26d). This is the "reach every entry path" backstop for encoding
/// tracing: unlike the `--trace-encoding` CLI flag (compile/check only), the
/// env var is read in [`Elaborator::new`], so it also fires on the doctor
/// oracle's per-module re-collection and any other path that builds an
/// `Elaborator` without threading trace flags.
///
/// Semantics mirror the flag: `*` or `all` (case-insensitive) traces every
/// type (`Some("")`, which [`Elaborator::should_trace_encoding`] treats as
/// trace-all); any other non-empty value traces that one type; unset or empty
/// is off. When the var is unset this returns `None`, so behaviour is
/// byte-identical to before for every non-debugging run.
pub(super) fn trace_encoding_from_env() -> Option<String> {
    map_trace_encoding_value(albite_runtime::settings::read("TRACE_ENCODING").as_deref())
}

/// Pure mapping from a raw `ALBITE_TRACE_ENCODING` value to a trace target.
/// Split out from the env read so the branch logic is unit-testable without
/// mutating process-global env state.
fn map_trace_encoding_value(raw: Option<&str>) -> Option<String> {
    match raw {
        None | Some("") => None,
        Some(value) if value.eq_ignore_ascii_case("*") || value.eq_ignore_ascii_case("all") => {
            Some(String::new())
        }
        Some(value) => Some(value.to_string()),
    }
}

#[cfg(test)]
mod trace_encoding_env_tests {
    use super::map_trace_encoding_value;

    #[test]
    fn unset_and_empty_are_off() {
        assert_eq!(map_trace_encoding_value(None), None);
        assert_eq!(map_trace_encoding_value(Some("")), None);
    }

    #[test]
    fn wildcard_and_all_trace_everything() {
        // Empty target = "trace all" per `should_trace_encoding`.
        assert_eq!(map_trace_encoding_value(Some("*")), Some(String::new()));
        assert_eq!(map_trace_encoding_value(Some("all")), Some(String::new()));
        assert_eq!(map_trace_encoding_value(Some("ALL")), Some(String::new()));
    }

    #[test]
    fn a_name_traces_that_type() {
        assert_eq!(
            map_trace_encoding_value(Some("TypeDef")),
            Some("TypeDef".to_string())
        );
    }

    /// Assert the env-reading wrapper actually consults the var and maps it.
    /// Set → read → remove → read within one test: no other test asserts on
    /// this var, so the transient global set only adds (harmless) tracing
    /// noise to any concurrent elaboration, never a failure.
    #[test]
    fn from_env_reads_and_maps_the_var() {
        std::env::set_var("ALBITE_TRACE_ENCODING", "TypeExpr");
        assert_eq!(
            super::trace_encoding_from_env(),
            Some("TypeExpr".to_string())
        );
        std::env::remove_var("ALBITE_TRACE_ENCODING");
        assert_eq!(super::trace_encoding_from_env(), None);
    }

    /// Each `CollectedElaborator` setter reaches the elaborator it wraps
    /// (moved here from `output/mod.rs` by ADR 2.10.26c §2.3).
    #[test]
    fn the_collected_setters_reach_the_elaborator() {
        let (ast, _) = crate::parse("fn one() -> Nat { 1 }");
        let mut ctx = albite_core::Context::new();
        let mut collected =
            crate::elaborate::collect_definitions(&ast, &mut ctx).expect("collects");
        collected.set_trace_target(Some("one".into()));
        collected.set_trace_encoding(Some("Pair".into()));
        collected.set_trace_normalization(Some("List".into()));
        assert_eq!(collected.elaborator.trace_target.as_deref(), Some("one"));
        assert_eq!(collected.elaborator.trace_encoding.as_deref(), Some("Pair"));
        assert_eq!(
            collected.elaborator.trace_normalization.as_deref(),
            Some("List")
        );

        // The bundle applies every option, mode and constructor tracing too.
        collected.apply_trace_options(&crate::driver::output::TraceOptions {
            trace_types: Some("two".into()),
            trace_encoding: Some("Triple".into()),
            trace_normalization: None,
            trace_ctor_registration: true,
            elab_mode: crate::elaborate::ElabMode::Test,
        });
        assert_eq!(collected.elaborator.trace_target.as_deref(), Some("two"));
        assert_eq!(
            collected.elaborator.trace_encoding.as_deref(),
            Some("Triple")
        );
        assert_eq!(collected.elaborator.trace_normalization, None);
        assert_eq!(
            collected.elaborator.elab_mode,
            crate::elaborate::ElabMode::Test
        );
        assert!(collected.elaborator.trace_ctor_registration);
        assert!(collected.elaborator.env.trace_ctor_registration);
    }
}

impl<'a> Elaborator<'a> {
    /// Set the trace target for --trace-types (ADR 13.4.26c §5).
    pub fn set_trace_target(&mut self, target: Option<String>) {
        self.trace_target = target;
    }

    /// Check if tracing is active for the current definition.
    pub(crate) fn should_trace(&self) -> bool {
        if let Some(ref target) = self.trace_target {
            if let Some(ref current) = self.current_def_name {
                return current == target;
            }
        }
        false
    }

    /// Emit a trace message if tracing is active.
    pub(crate) fn trace(&self, label: &str, message: &str) {
        if self.should_trace() {
            let def = self.current_def_name.as_deref().unwrap_or("<unknown>");
            eprintln!("[trace] {}: {}", def, label);
            for line in message.lines() {
                eprintln!("  {}", line);
            }
        }
    }

    /// Format a type with semantic annotation from provenance if available.
    ///
    /// For `μα_List. (Unit + (String × α_List))` with provenance, returns
    /// `"μα_List. (Unit + (String × α_List))  (semantic: List<String>)"`.
    pub(crate) fn format_type_with_provenance(&self, ty: &Type) -> String {
        let structural = format!("{}", ty);
        if let Type::Mu(binder, _) = ty {
            if let Some(origin) = self.type_provenance.mu_origins.get(binder) {
                let semantic = if origin.type_args.is_empty() {
                    origin.adt_name.clone()
                } else {
                    let args: Vec<String> =
                        origin.type_args.iter().map(|a| format!("{}", a)).collect();
                    format!("{}<{}>", origin.adt_name, args.join(", "))
                };
                return format!("{}  (semantic: {})", structural, semantic);
            }
        }
        structural
    }

    // ─── Encoding trace (--trace-encoding) ──────────────────────────

    /// Set the trace target for --trace-encoding (ADR 18.4.26h §3).
    ///
    /// Passing `None` clears any target, INCLUDING one seeded from
    /// `ALBITE_TRACE_ENCODING` by [`Elaborator::new`]. Callers that only want
    /// to apply a CLI flag when it is present must guard on `Some` so the env
    /// default survives (see `apply_trace_options`); the env var is the "reach
    /// every entry path" backstop (ADR 22.7.26d), including the doctor oracle,
    /// which threads no CLI flag.
    pub fn set_trace_encoding(&mut self, target: Option<String>) {
        self.trace_encoding = target;
    }

    /// Check if encoding tracing is active for a given type name.
    pub(crate) fn should_trace_encoding(&self, type_name: &str) -> bool {
        match self.trace_encoding {
            Some(ref target) if target.is_empty() => true, // trace all
            Some(ref target) => target == type_name,
            None => false,
        }
    }

    /// Emit an encoding trace message to stderr.
    pub(crate) fn trace_encoding(&self, tag: &str, message: &str) {
        if self.trace_encoding.is_some() {
            eprintln!("[{}] {}", tag, message);
        }
    }

    // ─── Normalization trace (--trace-normalization) ────────────────

    /// Set the trace target for --trace-normalization (ADR 20.4.26c).
    pub fn set_trace_normalization(&mut self, target: Option<String>) {
        self.trace_normalization = target;
    }

    /// Check if normalization tracing is active for a given type name.
    pub(crate) fn should_trace_normalization(&self, type_name: &str) -> bool {
        match self.trace_normalization {
            Some(ref target) if target.is_empty() => true,
            Some(ref target) => target == type_name,
            None => false,
        }
    }

    /// Emit a normalization trace message to stderr.
    pub(crate) fn trace_normalization(&self, message: &str) {
        if self.trace_normalization.is_some() {
            eprintln!("[norm] {}", message);
        }
    }
}

/// The same setters on a collected elaborator, which is what the driver holds
/// between the collection and body passes. Moved from `output/mod.rs`, whose
/// subject is the output, by ADR 2.10.26c §2.3.
impl super::CollectedElaborator<'_> {
    /// Set the trace target for --trace-types (ADR 13.4.26c §5).
    pub fn set_trace_target(&mut self, target: Option<String>) {
        self.elaborator.set_trace_target(target);
    }

    /// Set the trace target for --trace-encoding (ADR 18.4.26h §3).
    pub fn set_trace_encoding(&mut self, target: Option<String>) {
        self.elaborator.set_trace_encoding(target);
    }

    /// Set the trace target for --trace-normalization (ADR 20.4.26c).
    pub fn set_trace_normalization(&mut self, target: Option<String>) {
        self.elaborator.set_trace_normalization(target);
    }

    /// Apply all trace and mode options from a `TraceOptions` bundle.
    pub fn apply_trace_options(&mut self, trace: &crate::driver::output::TraceOptions) {
        self.set_trace_target(trace.trace_types.clone());
        // Only override the env-seeded default (ADR 22.7.26d) when the CLI flag
        // is actually present; an absent `--trace-encoding` must not wipe a
        // `ALBITE_TRACE_ENCODING` target.
        if let Some(target) = &trace.trace_encoding {
            self.set_trace_encoding(Some(target.clone()));
        }
        self.set_trace_normalization(trace.trace_normalization.clone());
        self.set_elab_mode(trace.elab_mode);
        self.elaborator.trace_ctor_registration = trace.trace_ctor_registration;
        self.elaborator.env.trace_ctor_registration = trace.trace_ctor_registration;
    }
}
