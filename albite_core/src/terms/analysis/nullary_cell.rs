//! The shared read-only cell of a nullary construction (ADR 1.10.26g D1, D2).
//!
//! An empty constructor of a recursive type stores the same bytes on every
//! execution, so codegen emits them once as a link-once constant and folds to
//! its address. The cell's name and payload bytes are pure functions of the
//! value, kept here so they are tested and mutated without LLVM; the global
//! itself is built in `albite_codegen`'s `sums/nullary_cell.rs`.

/// Width of a tag in a `{ i32, [N x i8] }` blob: a nested blob's tag sits at
/// the enclosing payload's offset 0, and its own payload starts this far on.
const TAG_BYTES: usize = 4;

/// The global's name, a pure function of the value (D2):
/// `__tg_nullary.<N>.<t0>_<t1>…`. `N` is the payload ARRAY length, not the
/// ABI size, because `[5 x i8]` and `[8 x i8]` blobs are both 12 bytes but
/// distinct LLVM types, and one symbol cannot carry two.
#[must_use]
pub fn nullary_cell_name(payload_len: u32, tag_path: &[u32]) -> String {
    let tags: Vec<String> = tag_path.iter().map(u32::to_string).collect();
    format!("__tg_nullary.{payload_len}.{}", tags.join("_"))
}

/// The payload bytes after the outer tag: each nested tag at a 4-byte stride
/// in the given byte order, zero elsewhere (D1). `None` when the nested tags
/// do not fit in `payload_len` bytes — codegen then folds to the heap.
#[must_use]
pub fn nullary_payload_bytes(
    payload_len: usize,
    nested: &[u32],
    little_endian: bool,
) -> Option<Vec<u8>> {
    if nested.len().checked_mul(TAG_BYTES)? > payload_len {
        return None;
    }
    let mut bytes = vec![0u8; payload_len];
    for (level, tag) in nested.iter().enumerate() {
        let encoded = if little_endian {
            tag.to_le_bytes()
        } else {
            tag.to_be_bytes()
        };
        let offset = level * TAG_BYTES;
        bytes[offset..offset + TAG_BYTES].copy_from_slice(&encoded);
    }
    Some(bytes)
}

/// Whether an LLVM data-layout string describes a little-endian target: an
/// `E` component means big-endian, and LLVM's default, absent either, is
/// little-endian.
#[must_use]
pub fn data_layout_is_little_endian(layout: &str) -> bool {
    !layout.split('-').any(|component| component == "E")
}
