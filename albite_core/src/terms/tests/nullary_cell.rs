//! ADR 1.10.26g AC 2: the cell's payload bytes and its name are pure and
//! pinned.

use crate::terms::analysis::{
    data_layout_is_little_endian, nullary_cell_name, nullary_payload_bytes,
};

#[test]
fn payload_bytes_place_nested_tags_at_four_byte_strides() {
    let bytes = nullary_payload_bytes(16, &[1, 2, 3], true).expect("fits");
    assert_eq!(
        bytes,
        vec![1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0],
        "nested tags at 0, 4, 8; zero after"
    );
    // No nested tags: an all-zero payload of the requested length.
    assert_eq!(nullary_payload_bytes(5, &[], true), Some(vec![0; 5]));
    assert_eq!(nullary_payload_bytes(0, &[], false), Some(vec![]));
}

#[test]
fn payload_bytes_honour_byte_order() {
    let tag = 0x0102_0304;
    assert_eq!(
        nullary_payload_bytes(8, &[tag], true),
        Some(vec![4, 3, 2, 1, 0, 0, 0, 0])
    );
    assert_eq!(
        nullary_payload_bytes(8, &[tag], false),
        Some(vec![1, 2, 3, 4, 0, 0, 0, 0])
    );
}

#[test]
fn payload_bytes_refuse_a_path_that_does_not_fit() {
    // Exactly full fits; one byte short does not.
    assert!(nullary_payload_bytes(8, &[1, 1], true).is_some());
    assert_eq!(nullary_payload_bytes(7, &[1, 1], true), None);
    assert_eq!(nullary_payload_bytes(0, &[0], true), None);
    assert_eq!(nullary_payload_bytes(3, &[1], false), None);
}

#[test]
fn cell_name_is_a_function_of_length_and_path() {
    assert_eq!(nullary_cell_name(16, &[0]), "__tg_nullary.16.0");
    assert_eq!(nullary_cell_name(16, &[1, 0, 2]), "__tg_nullary.16.1_0_2");
    // Equal inputs, equal names.
    assert_eq!(nullary_cell_name(8, &[2, 1]), nullary_cell_name(8, &[2, 1]));
    // Two lengths with the same 12-byte ABI size are different LLVM types.
    assert_ne!(nullary_cell_name(5, &[0]), nullary_cell_name(8, &[0]));
    // Different paths, including ones whose digits concatenate alike.
    assert_ne!(nullary_cell_name(8, &[1, 0]), nullary_cell_name(8, &[0, 1]));
    assert_ne!(
        nullary_cell_name(8, &[1, 12]),
        nullary_cell_name(8, &[11, 2])
    );
    assert_ne!(nullary_cell_name(8, &[1]), nullary_cell_name(8, &[1, 0]));
}

#[test]
fn data_layout_byte_order_reads_the_endianness_component() {
    assert!(data_layout_is_little_endian(
        "e-m:o-i64:64-i128:128-n32:64-S128"
    ));
    assert!(!data_layout_is_little_endian("E-m:e-i64:64-n32:64-S128"));
    assert!(!data_layout_is_little_endian("m:e-E-i64:64"));
    // LLVM's default, absent either component, is little-endian.
    assert!(data_layout_is_little_endian(""));
    // An `E` inside another component is not the endianness flag.
    assert!(data_layout_is_little_endian("e-m:E-i64:64"));
}
