//! Tests for the region copy layouts against the code generator (ADR
//! 13.10.26q AC 3): its ADT table as `NamedTypes`, the kinds the runtime
//! reads, sizes and offsets against LLVM's `TargetData`, and one end-to-end
//! copy through `albite_runtime`'s walker. `tests.rs` holds the LLVM-free
//! half; the two were split when formatting took one file over the size cap.

use super::tests::{layout_of, list_of_nat, var};
use super::{encode, CopyNode, NamedTypes};
use crate::codegen::CodeGen;
use albite_core::types::Type;
use inkwell::context::Context;
use inkwell::types::BasicTypeEnum;

// 13.10.26q D4: the code generator's own ADT table answers `NamedTypes`, its
// payloads substituted the way `lower_app` encodes them.
#[test]
fn type_lowering_answers_an_adts_payloads() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "region_layout_adts");
    let constructor = |name: &str, fields: Vec<Type>, index| crate::types::CodegenConstructor {
        name: name.into(),
        fields,
        index,
    };
    codegen.types.adt_types.insert(
        "Opt".into(),
        (
            vec!["T".into()],
            vec![
                constructor("None", vec![], 0),
                constructor("Some", vec![var("T")], 1),
            ],
        ),
    );
    assert_eq!(
        NamedTypes::adt_payloads(&codegen.types, "Opt", &[Type::String]),
        Some(vec![Type::Unit, Type::String])
    );
    assert_eq!(
        NamedTypes::adt_payloads(&codegen.types, "Absent", &[]),
        None
    );
}

// 13.10.26q D4: the kinds the encoder writes are the kinds the runtime reads.
#[test]
fn the_kinds_match_the_runtime() {
    use albite_runtime::copy_out as runtime;
    assert_eq!(super::KIND_STRING, runtime::KIND_STRING);
    assert_eq!(super::KIND_STRUCT, runtime::KIND_STRUCT);
    assert_eq!(super::KIND_TAGGED, runtime::KIND_TAGGED);
    assert_eq!(super::KIND_BOX, runtime::KIND_BOX);
    assert_eq!(super::NO_NODE, runtime::NO_NODE);
}

/// The lowered type and its `TargetData` store size, through the same
/// `TypeLowering` the code generator compiles with.
fn lowered<'ctx>(codegen: &mut CodeGen<'ctx>, ty: &Type) -> (BasicTypeEnum<'ctx>, u64) {
    let lowered = codegen.types.lower_type(ty);
    (lowered, codegen.types.type_size(lowered))
}

// 13.10.26q AC3: every shape's size is LLVM's, and the struct and blob
// offsets the layout encodes are the ones `TargetData` gives the lowered type.
#[test]
fn sizes_and_offsets_match_target_data() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "region_layout_offsets");
    for ty in [
        Type::Nat,
        Type::Bool,
        Type::Unit,
        Type::String,
        Type::product(Type::Nat, Type::String),
        Type::product(Type::Bool, Type::String),
        Type::sum(Type::Nat, Type::String),
        Type::sum(Type::Bool, Type::Unit),
        Type::product(Type::String, Type::sum(Type::Nat, Type::String)),
        list_of_nat(),
    ] {
        let (_, size) = lowered(&mut codegen, &ty);
        assert_eq!(layout_of(&ty).size, size, "{ty}");
    }
    let pair = codegen
        .types
        .lower_type(&Type::product(Type::Bool, Type::String))
        .into_struct_type();
    let blob = codegen
        .types
        .lower_type(&Type::sum(Type::Nat, Type::String))
        .into_struct_type();
    let target = codegen
        .types
        .target_data
        .as_ref()
        .expect("native target data");
    let pair_layout = layout_of(&Type::product(Type::Bool, Type::String));
    let CopyNode::Struct(fields) = &pair_layout.nodes[pair_layout.root.unwrap()] else {
        panic!("a struct");
    };
    assert_eq!(fields[0].0, target.offset_of_element(&pair, 1).unwrap());
    assert_eq!(
        super::BLOB_PAYLOAD_OFFSET,
        target.offset_of_element(&blob, 1).unwrap()
    );
    // A list's block is the unfolded sum's blob, as `fold_to_heap` sizes it.
    let block = codegen.types.lower_type(&Type::sum(
        Type::Unit,
        Type::product(Type::Nat, list_of_nat()),
    ));
    let block_size = codegen.types.type_size(block);
    let layout = layout_of(&list_of_nat());
    assert!(
        matches!(layout.nodes[layout.root.unwrap()], CopyNode::Box { size, .. } if size == block_size)
    );
}

// 13.10.26q AC2/AC3: a layout the code generator encodes is one the runtime
// walks — a region String copied out through `albite_runtime`.
#[test]
fn the_runtime_copies_through_an_encoded_layout() {
    let words = encode(&layout_of(&Type::product(Type::Nat, Type::String))).unwrap();
    let mut region_bytes = *b"region!!";
    let region = region_bytes.as_mut_ptr() as usize;
    let mut slot = [7u64, region as u64, 8];
    let mut copies: Vec<Box<[u8]>> = Vec::new();
    // SAFETY: `slot` is a `(Nat, String)` whose string lies in `region_bytes`.
    let tally = unsafe {
        albite_runtime::copy_out::copy_out_with(
            slot.as_mut_ptr().cast(),
            &words,
            &|addr| (region..region + 8).contains(&addr),
            &mut |size| {
                copies.push(vec![0u8; size].into_boxed_slice());
                copies.last_mut().unwrap().as_mut_ptr()
            },
        )
    }
    .unwrap();
    assert_eq!(tally.copied, 1);
    assert_ne!(slot[1], region as u64);
    region_bytes.fill(0xA5);
    assert_eq!(&*copies[0], b"region!!");
    assert_eq!(slot[0], 7);
}
