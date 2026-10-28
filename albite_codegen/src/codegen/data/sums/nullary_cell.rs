//! The shared read-only cell a nullary fold returns (ADR 1.10.26g D1, D2).
//!
//! An empty constructor of a recursive type — `LNil()`, `TEmpty()` — stores
//! the same bytes on every execution: its tag path, then bytes no `match`
//! reads. So instead of allocating, a fold of one returns the address of a
//! `linkonce_odr hidden unnamed_addr constant`, named from its contents, which
//! the linker keeps once across codegen units. Nothing in either compiler or
//! the runtime writes, frees or compares the address of a fold cell (D3), so
//! sharing is unobservable; the read-only section is a backstop.
//!
//! The name and payload bytes are pure, in
//! `albite_core::terms::analysis::nullary_cell`; this file only builds the
//! global and declines what it cannot encode.

use crate::codegen::abi::blob_payload_len;
use crate::codegen::CodeGen;
use albite_core::terms::analysis::{
    data_layout_is_little_endian, nullary_cell_name, nullary_payload_bytes,
};
use inkwell::module::Linkage;
use inkwell::types::{BasicTypeEnum, StructType};
use inkwell::values::{GlobalValue, PointerValue};
use inkwell::{AddressSpace, GlobalVisibility};

/// Whether the module's target uses comdats for link-once data. Mach-O has
/// none and coalesces weak definitions by name; ELF and COFF want one, as
/// clang gives `linkonce_odr` data.
pub(crate) fn target_uses_comdats(triple: &str) -> bool {
    !["apple", "darwin", "macos", "ios"]
        .iter()
        .any(|vendor| triple.contains(vendor))
}

impl<'ctx> CodeGen<'ctx> {
    /// The shared read-only cell holding a nullary construction with this
    /// tag path, or `None` when the lowered type is not `{ i32, [N x i8] }`
    /// or the nested tags do not fit its payload (D1); the caller then folds
    /// to the heap as before, and a profiled build stamps it `MuNullary`.
    pub(crate) fn nullary_cell(
        &mut self,
        sum_llvm_ty: BasicTypeEnum<'ctx>,
        tag_path: &[u32],
    ) -> Option<PointerValue<'ctx>> {
        let BasicTypeEnum::StructType(struct_ty) = sum_llvm_ty else {
            return None;
        };
        let (Some(payload_len), Some((&outer_tag, nested))) =
            (blob_payload_len(struct_ty), tag_path.split_first())
        else {
            return None;
        };
        let name = nullary_cell_name(payload_len, tag_path);
        if let Some(existing) = self.module.get_global(&name) {
            return Some(existing.as_pointer_value());
        }
        let little_endian =
            data_layout_is_little_endian(&self.module.get_data_layout().as_str().to_string_lossy());
        let bytes = nullary_payload_bytes(payload_len as usize, nested, little_endian)?;
        let global = self.add_nullary_global(struct_ty, &name, outer_tag, &bytes);
        Some(global.as_pointer_value())
    }

    /// Emit the cell: `{ i32 outer_tag, [N x i8] bytes }`, read-only,
    /// link-once, hidden, aligned 16 as `fold_to_heap`'s store is.
    fn add_nullary_global(
        &mut self,
        struct_ty: StructType<'ctx>,
        name: &str,
        outer_tag: u32,
        bytes: &[u8],
    ) -> GlobalValue<'ctx> {
        let i8_type = self.context.i8_type();
        let byte_values: Vec<_> = bytes
            .iter()
            .map(|byte| i8_type.const_int(u64::from(*byte), false))
            .collect();
        let initializer = struct_ty.const_named_struct(&[
            self.context
                .i32_type()
                .const_int(u64::from(outer_tag), false)
                .into(),
            i8_type.const_array(&byte_values).into(),
        ]);
        let global = self
            .module
            .add_global(struct_ty, Some(AddressSpace::default()), name);
        global.set_initializer(&initializer);
        global.set_constant(true);
        global.set_linkage(Linkage::LinkOnceODR);
        global.set_visibility(GlobalVisibility::Hidden);
        global.set_unnamed_address(inkwell::values::UnnamedAddress::Global);
        global.set_alignment(16);
        let triple = self.module.get_triple();
        if target_uses_comdats(&triple.as_str().to_string_lossy()) {
            global.set_comdat(self.module.get_or_insert_comdat(name));
        }
        global
    }
}

#[cfg(test)]
mod tests {
    use super::target_uses_comdats;
    use crate::codegen::abi::blob_payload_len;
    use inkwell::context::Context;

    /// ADR 1.10.26g D1: only `{ i32, [N x i8] }` has a cell layout — a wider
    /// tag, a non-byte payload, or a field count other than two declines.
    #[test]
    fn only_an_i32_tag_over_a_byte_array_is_a_blob() {
        let context = Context::create();
        let i32_type = context.i32_type();
        let i64_type = context.i64_type();
        let bytes = context.i8_type().array_type(16);
        let blob = context.struct_type(&[i32_type.into(), bytes.into()], false);
        assert_eq!(blob_payload_len(blob), Some(16));
        let wide_tag = context.struct_type(&[i64_type.into(), bytes.into()], false);
        assert_eq!(blob_payload_len(wide_tag), None);
        let words = context.struct_type(&[i32_type.into(), i32_type.array_type(4).into()], false);
        assert_eq!(blob_payload_len(words), None);
        let no_array = context.struct_type(&[i32_type.into(), i64_type.into()], false);
        assert_eq!(blob_payload_len(no_array), None);
        let tag_only = context.struct_type(&[i32_type.into()], false);
        assert_eq!(blob_payload_len(tag_only), None);
    }

    #[test]
    fn comdats_are_for_every_target_but_mach_o() {
        assert!(target_uses_comdats("aarch64-unknown-linux-gnu"));
        assert!(target_uses_comdats("x86_64-pc-windows-msvc"));
        assert!(!target_uses_comdats("arm64-apple-darwin23.6.0"));
        assert!(!target_uses_comdats("x86_64-apple-macosx14.0.0"));
    }
}
