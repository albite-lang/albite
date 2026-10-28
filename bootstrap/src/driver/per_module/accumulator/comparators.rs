//! Comparator synthesis on the assembled definition set (ADR 11.8.26b §2.3).
//!
//! Moved out of `accumulator/mod.rs` unchanged (ADR 28.9.26d) to keep that file
//! under its size threshold.

use super::ModuleTreeAccumulator;

impl ModuleTreeAccumulator {
    /// Synthesize the comparators `__compare` emitted but nothing defines, so
    /// they are in the definition set *before* the gate runs (ADR 11.8.26b §2.3).
    ///
    /// They used to be appended to `ElabOutput.defs` after `admit_or_reject`,
    /// which made them a second route into the trusted environment — the exact
    /// hole ADR 29.6.26e §2.5 exists to close. They are structurally recursive
    /// by construction, so this is not a fix for a live unsoundness; it is the
    /// removal of an unchecked path that a later synthesizer change could make
    /// into one.
    ///
    /// Returns how many were added, for the verbose line the callers print.
    pub(in crate::driver::per_module) fn synthesize_comparators(&mut self) -> usize {
        let types = crate::comparator::ComparatorTypes::new(
            self.type_meta.record_types.clone(),
            &self.type_meta.encoded_types,
            &self.type_meta.type_provenance,
            self.type_meta.adt_types.clone(),
            &self.type_meta.mutual_recursion_groups,
        );
        let synthesized =
            crate::comparator::discover::synth_missing_comparators(&self.defs, &types);
        let count = synthesized.len();
        self.defs.extend(synthesized);
        count
    }
}
