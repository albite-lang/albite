//! Type-level metadata accumulated across modules — moved unchanged from
//! `accumulator/mod.rs` by ADR 2.10.26c (§2.3), to keep that file under its
//! size threshold as the kernel boundary joins it.

use std::collections::HashMap;

use crate::elaborate::{ElabOutput, TypeProvenance};
use albite_core::Type;

/// Type-level metadata accumulated across modules.
///
/// Groups the 6 type metadata maps to reduce field count on `ModuleTreeAccumulator`.
pub(in crate::driver::per_module) struct AccumulatedTypeMeta {
    pub(in crate::driver::per_module) record_types: HashMap<String, Vec<(String, Type)>>,
    pub(in crate::driver::per_module) adt_types:
        HashMap<String, (Vec<String>, Vec<crate::elaborate::Constructor>)>,
    pub(in crate::driver::per_module) type_aliases: HashMap<String, (Vec<String>, Type)>,
    pub(in crate::driver::per_module) type_provenance: TypeProvenance,
    pub(in crate::driver::per_module) encoded_types: HashMap<String, Type>,
    pub(in crate::driver::per_module) mutual_recursion_groups: HashMap<String, Vec<String>>,
    pub(in crate::driver::per_module) type_visibilities: HashMap<String, crate::ast::Visibility>,
    pub(in crate::driver::per_module) record_field_visibilities:
        HashMap<String, Vec<Option<crate::ast::Visibility>>>,
}

impl AccumulatedTypeMeta {
    pub(in crate::driver::per_module) fn new() -> Self {
        Self {
            record_types: HashMap::new(),
            adt_types: HashMap::new(),
            type_aliases: HashMap::new(),
            type_provenance: TypeProvenance::default(),
            encoded_types: HashMap::new(),
            mutual_recursion_groups: HashMap::new(),
            type_visibilities: HashMap::new(),
            record_field_visibilities: HashMap::new(),
        }
    }

    pub(in crate::driver::per_module) fn merge_from(&mut self, output: &mut ElabOutput) {
        self.record_types
            .extend(std::mem::take(&mut output.record_types));
        self.adt_types.extend(std::mem::take(&mut output.adt_types));
        self.type_aliases
            .extend(std::mem::take(&mut output.type_aliases));
        self.type_provenance
            .mu_origins
            .extend(std::mem::take(&mut output.type_provenance.mu_origins));
        self.encoded_types
            .extend(std::mem::take(&mut output.encoded_types));
        self.mutual_recursion_groups
            .extend(std::mem::take(&mut output.mutual_recursion_groups));
        self.type_visibilities
            .extend(std::mem::take(&mut output.type_visibilities));
        self.record_field_visibilities
            .extend(std::mem::take(&mut output.record_field_visibilities));
    }
}
