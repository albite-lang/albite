//! Tests for the region copy layouts (ADR 13.10.26q AC 3): `copy_layout`
//! over P0.3's shape table with no LLVM, its sizes and offsets against
//! LLVM's `TargetData`, the encoding the runtime reads, and one end-to-end
//! copy through `albite_runtime`'s walker.

use std::collections::HashMap;

use super::{copy_layout, encode, layout_symbol, CopyNode, NamedTypes, Unclassified};
use albite_core::types::Type;

/// An ADT and record table for the tests.
#[derive(Default)]
struct Table {
    adts: HashMap<String, Vec<Type>>,
    records: HashMap<String, Type>,
}

impl NamedTypes for Table {
    fn record(&self, name: &str) -> Option<Type> {
        self.records.get(name).cloned()
    }

    fn adt_payloads(&self, name: &str, _args: &[Type]) -> Option<Vec<Type>> {
        self.adts.get(name).cloned()
    }
}

pub(super) fn var(name: &str) -> Type {
    Type::TyVar(name.into())
}

fn mu(binder: &str, body: Type) -> Type {
    Type::Mu(binder.into(), Box::new(body))
}

/// `List<Nat>` as the elaborator encodes it.
pub(super) fn list_of_nat() -> Type {
    mu(
        "α_List",
        Type::sum(Type::Unit, Type::product(Type::Nat, var("α_List"))),
    )
}

/// `Tree = Node(Nat, Forest)`, `Forest = FNil | FCons(Tree, Forest)`.
fn tree_forest() -> Table {
    let mut table = Table::default();
    table
        .adts
        .insert("Tree".into(), vec![Type::product(Type::Nat, var("Forest"))]);
    table.adts.insert(
        "Forest".into(),
        vec![Type::Unit, Type::product(var("Tree"), var("Forest"))],
    );
    table
}

pub(super) fn layout_of(ty: &Type) -> super::CopyLayout {
    copy_layout(ty, &Table::default()).unwrap()
}

// 13.10.26q AC3: a register scalar, and a struct or sum of them, holds no
// pointer, so it has no layout and needs no copy.
#[test]
fn pointer_free_values_have_no_layout() {
    for ty in [
        Type::Nat,
        Type::Int,
        Type::Bool,
        Type::Unit,
        Type::product(Type::Nat, Type::Bool),
        Type::sum(Type::Nat, Type::Nat),
    ] {
        let layout = layout_of(&ty);
        assert_eq!(layout.root, None, "{ty}");
        assert_eq!(encode(&layout), None, "{ty}");
    }
}

// 13.10.26q AC3: a String is one node, and its words are the runtime's.
#[test]
fn a_string_is_one_string_node() {
    let layout = layout_of(&Type::String);
    assert_eq!(layout.size, 16);
    assert_eq!(encode(&layout), Some(vec![2, super::KIND_STRING]));
}

// 13.10.26q AC3: a tuple places its String after the Nat, at offset 8.
#[test]
fn a_tuple_places_its_string_at_offset_eight() {
    let layout = layout_of(&Type::product(Type::Nat, Type::String));
    assert_eq!(layout.size, 24);
    let root = layout.root.unwrap();
    assert!(
        matches!(&layout.nodes[root], CopyNode::Struct(fields) if fields.len() == 1 && fields[0].0 == 8)
    );
}

// 13.10.26q AC3: `Result<Nat, String>` is the blob, each arm at its own size.
#[test]
fn a_sum_is_a_blob_with_one_size_per_arm() {
    let layout = layout_of(&Type::sum(Type::Nat, Type::String));
    assert_eq!(layout.size, 20);
    let root = layout.root.unwrap();
    let CopyNode::Tagged(variants) = &layout.nodes[root] else {
        panic!("{layout:?}");
    };
    assert_eq!(variants[0], (8, None));
    assert_eq!(variants[1].0, 16);
    assert!(variants[1].1.is_some());
}

// 13.10.26q AC3: a recursive list is a pointer to a 20-byte block whose
// Cons tail points back at the same box — a finite graph.
#[test]
fn a_list_is_a_box_whose_tail_points_back_at_itself() {
    let layout = layout_of(&list_of_nat());
    assert_eq!(layout.size, 8);
    let root = layout.root.unwrap();
    let CopyNode::Box {
        size: 20,
        content: Some(block),
    } = layout.nodes[root]
    else {
        panic!("{layout:?}");
    };
    let CopyNode::Tagged(variants) = &layout.nodes[block] else {
        panic!("{layout:?}");
    };
    let cons = variants[1].1.unwrap();
    assert_eq!(layout.nodes[cons], CopyNode::Struct(vec![(8, root)]));
}

// 13.10.26q AC3: a list nested twice shares one box, so the layout does not
// grow with the number of mentions.
#[test]
fn a_type_mentioned_twice_shares_one_box() {
    let layout = layout_of(&Type::product(list_of_nat(), list_of_nat()));
    let boxes = layout
        .nodes
        .iter()
        .filter(|node| matches!(node, CopyNode::Box { .. }))
        .count();
    assert_eq!(boxes, 1, "{layout:?}");
}

// 13.10.26q AC3, D4: in a mutual cluster the second binder is a marker for
// its member — a Tree's field points at a Forest block (20 bytes), never at
// another Tree (16), which reading the encoding as closed would give.
#[test]
fn a_mutual_cluster_marker_resolves_to_its_member() {
    let tree = mu(
        "α_Tree",
        mu("α_Forest", Type::product(Type::Nat, var("α_Forest"))),
    );
    let layout = copy_layout(&tree, &tree_forest()).unwrap();
    let root = layout.root.unwrap();
    let CopyNode::Box {
        size: 16,
        content: Some(tree_block),
    } = layout.nodes[root]
    else {
        panic!("{layout:?}");
    };
    let CopyNode::Struct(fields) = &layout.nodes[tree_block] else {
        panic!("{layout:?}");
    };
    let forest = fields[0].1;
    assert!(
        matches!(layout.nodes[forest], CopyNode::Box { size: 20, .. }),
        "{layout:?}"
    );
}

// 13.10.26q AC3: a flat ADT of three constructors is one blob of three arms.
#[test]
fn a_flat_adt_is_one_blob_of_all_its_arms() {
    let color = Type::adt(
        "Color",
        vec![],
        vec![
            ("Red".into(), Type::Unit),
            ("Green".into(), Type::Nat),
            ("Blue".into(), Type::String),
        ],
    );
    let layout = layout_of(&color);
    assert_eq!(layout.size, 20);
    let CopyNode::Tagged(variants) = &layout.nodes[layout.root.unwrap()] else {
        panic!("{layout:?}");
    };
    assert_eq!(variants.len(), 3);
}

// 13.10.26q AC3: a named single-constructor record type is its payload, and
// a record name expands to its product.
#[test]
fn named_types_expand_through_the_table() {
    let mut table = Table::default();
    table.adts.insert(
        "Row".into(),
        vec![Type::product(
            Type::Nat,
            Type::product(Type::String, list_of_nat()),
        )],
    );
    table
        .records
        .insert("Point".into(), Type::product(Type::Nat, Type::Nat));
    let row = copy_layout(&var("@Row"), &table).unwrap();
    assert_eq!(row.size, 32);
    let point = copy_layout(&var("Point"), &table).unwrap();
    assert_eq!((point.size, point.root), (16, None));
}

// 13.10.26q D11: a function, a reference or an unknown name has no layout,
// and the refusal names the shape.
#[test]
fn unclassifiable_shapes_are_named() {
    let closure = Type::arrow(Type::Nat, Type::Nat);
    assert_eq!(
        copy_layout(&closure, &Table::default()),
        Err(Unclassified {
            shape: closure.to_string()
        })
    );
    assert!(copy_layout(&var("Nowhere"), &Table::default()).is_err());
    assert!(copy_layout(&var("α_Nowhere"), &Table::default()).is_err());
}

// 13.10.26q D4: the symbol is a hash of the words — equal words, one name;
// different words, different names.
#[test]
fn the_symbol_names_the_words() {
    let string = encode(&layout_of(&Type::String)).unwrap();
    let list = encode(&layout_of(&list_of_nat())).unwrap();
    assert_eq!(layout_symbol(&string), layout_symbol(&string.clone()));
    assert_ne!(layout_symbol(&string), layout_symbol(&list));
    assert!(layout_symbol(&string).starts_with("__tg_region_layout."));
    assert_eq!(
        layout_symbol(&string).len(),
        "__tg_region_layout.".len() + 16
    );
}

// 13.10.26q D4: the symbol is FNV-1a over the words' little-endian bytes, so
// it is stable across compilers and runs.
#[test]
fn the_symbol_is_fnv1a_of_the_words() {
    assert_eq!(
        layout_symbol(&[2, super::KIND_STRING]),
        "__tg_region_layout.072184407c3a4ac6"
    );
}

// 13.10.26q D4: every node kind's width places the next node's words — the
// String before a list, the list's box, blob and struct — exactly where the
// runtime's walk reads them.
#[test]
fn a_string_and_a_list_encode_to_the_runtimes_words() {
    use super::{KIND_BOX, KIND_STRING, KIND_STRUCT, KIND_TAGGED, NO_NODE};
    let words = encode(&layout_of(&Type::product(Type::String, list_of_nat()))).unwrap();
    #[rustfmt::skip]
    let expected = vec![
        22,
        KIND_STRUCT, 2, 0, 7, 16, 8,          // word 1: (String, List<Nat>)
        KIND_STRING,                          // word 7
        KIND_BOX, 20, 15,                     // word 8: the list pointer
        KIND_STRUCT, 1, 8, 8,                 // word 11: Cons's tail is the box
        KIND_TAGGED, 4, 2, 0, NO_NODE, 16, 11, // word 15: Nil | Cons
    ];
    assert_eq!(words, expected);

    // A blob of three arms is 3 + 2 × 3 words, which no two-arm blob tells
    // apart from 3 + 2 + 3.
    let mut table = Table::default();
    table
        .adts
        .insert("Tri".into(), vec![Type::Unit, Type::String, Type::Nat]);
    let three_arms = encode(&copy_layout(&var("@Tri"), &table).unwrap()).unwrap();
    assert_eq!(
        three_arms.len(),
        1 + 9 + 1,
        "the length word, the blob, the string"
    );
    assert_eq!(three_arms[0] as usize, three_arms.len());
    assert_eq!(three_arms[10], KIND_STRING);
}
