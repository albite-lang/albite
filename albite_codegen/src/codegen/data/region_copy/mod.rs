//! The region copy-out's static layouts (ADR 13.10.26q D4).
//!
//! [`copy_layout`] maps a type to the shape its lowered value has; [`encode`]
//! writes that graph as the `u64` words `__albite_region_copy_out` walks
//! (`albite_runtime::copy_out` documents the format); and
//! [`CodeGen::region_copy_layout`] emits them once per module as
//! `__tg_region_layout.<fp>`, a `linkonce_odr hidden unnamed_addr constant`
//! on 1.10.26g's naming model. `<fp>` hashes the encoded words themselves —
//! never `Display`, which strips `@`, nor a `TypeHandle`, which is
//! phase-local — so two different layouts can never share a name.

mod layout;
mod type_names;

#[cfg(test)]
use layout::Unclassified;
pub use layout::{copy_layout, CopyLayout, CopyNode, NamedTypes, BLOB_PAYLOAD_OFFSET};

use crate::codegen::backend::CodeGenError;
use crate::codegen::CodeGen;
use crate::types::TypeLowering;
use albite_core::types::Type;
use inkwell::module::Linkage;
use inkwell::values::PointerValue;
use inkwell::{AddressSpace, GlobalVisibility};

/// Node kinds, equal to the runtime's `KIND_*` (a test pins them).
pub const KIND_STRING: u64 = 1;
pub const KIND_STRUCT: u64 = 2;
pub const KIND_TAGGED: u64 = 3;
pub const KIND_BOX: u64 = 4;
pub const NO_NODE: u64 = u64::MAX;

/// The words of `layout`: word 0 the length, the root's node at word 1, the
/// rest in index order. `None` for a value that holds no pointer.
#[must_use]
pub fn encode(layout: &CopyLayout) -> Option<Vec<u64>> {
    let root = layout.root?;
    // The root first, so it sits at word 1; then every other node in order.
    let order: Vec<usize> = std::iter::once(root)
        .chain((0..layout.nodes.len()).filter(|index| *index != root))
        .collect();
    let mut offsets = vec![0u64; layout.nodes.len()];
    let mut next = 1u64;
    for index in &order {
        offsets[*index] = next;
        next += node_words(&layout.nodes[*index]);
    }
    let at = |node: Option<usize>| node.map_or(NO_NODE, |index| offsets[index]);
    let mut words = vec![next];
    for index in order {
        match &layout.nodes[index] {
            CopyNode::String => words.push(KIND_STRING),
            CopyNode::Struct(fields) => {
                words.extend([KIND_STRUCT, fields.len() as u64]);
                for (offset, node) in fields {
                    words.extend([*offset, at(Some(*node))]);
                }
            }
            CopyNode::Tagged(variants) => {
                words.extend([KIND_TAGGED, BLOB_PAYLOAD_OFFSET, variants.len() as u64]);
                for (size, node) in variants {
                    words.extend([*size, at(*node)]);
                }
            }
            CopyNode::Box { size, content } => words.extend([KIND_BOX, *size, at(*content)]),
        }
    }
    Some(words)
}

fn node_words(node: &CopyNode) -> u64 {
    match node {
        CopyNode::String => 1,
        CopyNode::Struct(fields) => 2 + 2 * fields.len() as u64,
        CopyNode::Tagged(variants) => 3 + 2 * variants.len() as u64,
        CopyNode::Box { .. } => 3,
    }
}

/// `__tg_region_layout.<fp>`, `<fp>` the FNV-1a hash of the words.
#[must_use]
pub fn layout_symbol(words: &[u64]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in words.iter().flat_map(|word| word.to_le_bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("__tg_region_layout.{hash:016x}")
}

impl NamedTypes for TypeLowering<'_> {
    fn record(&self, name: &str) -> Option<Type> {
        self.record_encoding(name)
    }

    fn adt_payloads(&self, name: &str, args: &[Type]) -> Option<Vec<Type>> {
        TypeLowering::adt_payloads(self, name, args)
    }
}

impl<'ctx> CodeGen<'ctx> {
    /// The layout constant for a value of `ty` whose lowered size is
    /// `lowered_size`, or `None` when the value holds no pointer and needs no
    /// copy — a register scalar's IR is unchanged (D5).
    ///
    /// # Errors
    ///
    /// D11's refusal for a shape the copy cannot classify, and an internal
    /// error when the layout and the lowered value disagree on size — which
    /// would make the copy read the slot at the wrong offsets.
    pub(crate) fn region_copy_layout(
        &mut self,
        ty: &Type,
        lowered_size: u64,
    ) -> Result<Option<PointerValue<'ctx>>, CodeGenError> {
        let layout = copy_layout(ty, &self.types).map_err(|unclassified| {
            CodeGenError::Unsupported(format!(
                "a region whose value holds `{}` has no copy layout (ADR 13.10.26q D11)",
                unclassified.shape
            ))
        })?;
        let Some(words) = encode(&layout) else {
            return Ok(None);
        };
        if layout.size != lowered_size {
            return Err(CodeGenError::Unsupported(format!(
                "internal: the region copy layout of `{ty}` is {} bytes but its value lowers to \
                 {lowered_size} (ADR 13.10.26q D4)",
                layout.size
            )));
        }
        let name = layout_symbol(&words);
        if let Some(existing) = self.module.get_global(&name) {
            return Ok(Some(existing.as_pointer_value()));
        }
        let i64_type = self.context.i64_type();
        let values: Vec<_> = words
            .iter()
            .map(|word| i64_type.const_int(*word, false))
            .collect();
        let array_type = i64_type.array_type(words.len() as u32);
        let global = self
            .module
            .add_global(array_type, Some(AddressSpace::default()), &name);
        global.set_initializer(&i64_type.const_array(&values));
        global.set_constant(true);
        global.set_linkage(Linkage::LinkOnceODR);
        global.set_visibility(GlobalVisibility::Hidden);
        global.set_unnamed_address(inkwell::values::UnnamedAddress::Global);
        global.set_alignment(8);
        let triple = self.module.get_triple();
        if crate::codegen::data::sums::target_uses_comdats(&triple.as_str().to_string_lossy()) {
            global.set_comdat(self.module.get_or_insert_comdat(&name));
        }
        Ok(Some(global.as_pointer_value()))
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_target_data;
