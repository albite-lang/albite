//! Round-trip, sharing and accounting tests for the arena type nodes.

use super::*;
use crate::terms::Term;

fn arena() -> Arena {
    Arena::new()
}

fn sample_types() -> Vec<Type> {
    vec![
        Type::Nat,
        Type::Int,
        Type::Error,
        Type::TyVar("@Cursor".into()),
        Type::arrow(Type::Nat, Type::Bool),
        Type::product(Type::TyVar("a".into()), Type::String),
        Type::sum(Type::Unit, Type::Void),
        Type::Forall(
            "t".into(),
            Box::new(Type::arrow(
                Type::TyVar("t".into()),
                Type::TyVar("t".into()),
            )),
        ),
        Type::Mu(
            "α_List".into(),
            Box::new(Type::sum(
                Type::Unit,
                Type::product(Type::Nat, Type::TyVar("α_List".into())),
            )),
        ),
        Type::eq(Type::Nat, Term::Zero, Term::Succ(Box::new(Term::Zero))),
        Type::ptr(Type::Nat),
        Type::ref_ty(Type::Bool),
        Type::app("Forest", vec![Type::TyVar("t".into())]),
        Type::adt(
            "Tree",
            vec![Type::Nat],
            vec![("Leaf".into(), Type::Unit), ("Node".into(), Type::Nat)],
        ),
        Type::pi(
            "n",
            Type::Nat,
            Type::eq(Type::Nat, Term::Var("n".into()), Term::Zero),
        ),
        Type::sigma(
            "k",
            Type::Nat,
            Type::eq(Type::Nat, Term::Var("k".into()), Term::Zero),
        ),
        // The predicate binders (ADR 6.10.26h D10)
        Type::pred_app("P", vec![Term::Zero, Term::Var("n".into())]),
        Type::forall_pred("P", vec![Type::Nat], Type::pred_app("P", vec![Term::Zero])),
        Type::pred_abs(
            vec![("n".into(), Type::Nat)],
            Type::eq(Type::Nat, Term::Var("n".into()), Term::Zero),
        ),
    ]
}

/// 6.10.26h AC1 (D10): each predicate form reads as its own tag, 19–21,
/// substitutes through the owned hereditary rule, and accounts its strings.
#[test]
fn the_predicate_forms_are_their_own_nodes_and_tags() {
    use crate::ffi::types::node_substitute::substitute_type_handle;
    use crate::ffi::types::predicates::tg_type_tag;
    use crate::ffi::{tg_init, with_arena};
    tg_init();
    let samples = sample_types();
    let handles: Vec<_> = samples[samples.len() - 3..]
        .iter()
        .map(|ty| with_arena!(|arena| import_type(arena, ty)))
        .collect();
    let tags: Vec<u64> = handles.iter().map(|h| tg_type_tag(*h)).collect();
    assert_eq!(tags, vec![19, 20, 21]);
    let instantiated = with_arena!(|arena| {
        let predicate = import_type(arena, &samples[samples.len() - 1]);
        let app = substitute_type_handle(arena, handles[0], "P", predicate).expect("valid");
        materialize_type(arena, app).expect("valid")
    });
    assert_eq!(
        instantiated,
        Type::eq(Type::Nat, Term::Zero, Term::Zero),
        "only the first argument survives a one-parameter predicate's prefix"
    );
    let untouched = with_arena!(|arena| {
        let nat = import_type(arena, &Type::Nat);
        substitute_type_handle(arena, handles[1], "a", nat)
    });
    assert_eq!(
        untouched,
        Some(handles[1]),
        "an unchanged form keeps its handle"
    );
    let mut name = String::from("P");
    name.shrink_to_fit();
    let cap = name.capacity() as u64;
    assert_eq!(node_heap_bytes(&TypeNode::PredApp(name, Vec::new())), cap);
}

/// 6.10.26h D10: each predicate node's own strings and handle slabs, exactly;
/// non-zero names and two-element slabs pin every `+` and `*`.
#[test]
fn predicate_node_heap_bytes_are_exact() {
    use crate::ffi::test_support::sized_name;
    let handle = size_of::<TypeHandle>() as u64;
    let term_handle = size_of::<TermHandle>() as u64;
    let pair = size_of::<(String, TypeHandle)>() as u64;
    assert_eq!(
        node_heap_bytes(&TypeNode::PredApp(sized_name("Pqr"), vec![0, 1])),
        3 + 2 * term_handle
    );
    assert_eq!(
        node_heap_bytes(&TypeNode::ForallPred(sized_name("Pq"), vec![0, 1], 2)),
        2 + 2 * handle
    );
    assert_eq!(
        node_heap_bytes(&TypeNode::PredAbs(
            vec![(sized_name("n"), 0), (sized_name("mmm"), 1)],
            2
        )),
        2 * pair + 1 + 3
    );
}

/// 4.10.26h AC2 (D6): a `Sigma` imports as its own node, reads as tag 18 —
/// never a product's 7 — and equals only a `Sigma` up to renaming.
#[test]
fn a_sigma_is_its_own_node_and_tag() {
    use crate::ffi::types::node_equality::type_handles_equal;
    use crate::ffi::types::predicates::{tg_type_is_product, tg_type_tag};
    use crate::ffi::{tg_init, with_arena};
    tg_init();
    let body = |k: &str| Type::eq(Type::Nat, Term::Var(k.into()), Term::Zero);
    let import = |ty: &Type| with_arena!(|arena| import_type(arena, ty));
    let sigma = import(&Type::sigma("k", Type::Nat, body("k")));
    let renamed = import(&Type::sigma("j", Type::Nat, body("j")));
    let product = import(&Type::product(Type::Nat, body("k")));
    assert_eq!(tg_type_tag(sigma), 18);
    assert!(!tg_type_is_product(sigma));
    assert!(crate::ffi::with_arena_ref!(|arena| matches!(
        arena.get_type_node(sigma),
        Some(TypeNode::Sigma(x, _, _)) if x == "k"
    )));
    let equal = |a, b| crate::ffi::with_arena_ref!(|arena| type_handles_equal(arena, a, b));
    assert!(equal(sigma, renamed));
    assert!(!equal(sigma, product));
    let mut name = String::from("k");
    name.shrink_to_fit();
    let cap = name.capacity() as u64;
    assert_eq!(node_heap_bytes(&TypeNode::Sigma(name, 0, 1)), cap);
}

#[test]
fn import_materialize_round_trips_every_variant() {
    let mut arena = arena();
    for ty in sample_types() {
        let handle = import_type(&mut arena, &ty);
        let back = materialize_type(&arena, handle).expect("valid handle");
        assert_eq!(back, ty, "round-trip mismatch for {ty:?}");
    }
}

#[test]
fn materialize_rejects_dangling_handles() {
    let arena = arena();
    assert_eq!(materialize_type(&arena, 0), None);
    assert_eq!(materialize_type(&arena, crate::ffi::INVALID_HANDLE), None);
}

#[test]
fn composing_shares_children_instead_of_copying() {
    let mut arena = arena();
    let leaf = import_type(&mut arena, &Type::Nat);
    let nodes_after_leaf = arena.types.len();
    // Build a 64-deep chain of arrows over the same leaf. Owned-tree
    // storage retained O(2^depth) bytes for this shape; nodes must add
    // exactly ONE node per compose step.
    let mut current = leaf;
    for _ in 0..64 {
        let node = TypeNode::Arrow(current, current);
        current = arena.alloc_type_node(node);
    }
    assert_eq!(arena.types.len(), nodes_after_leaf + 64);
}

#[test]
fn node_heap_bytes_counts_strings_and_slabs() {
    let mut name = String::from("Forest");
    name.shrink_to_fit();
    let name_cap = name.capacity() as u64;
    let args = vec![0 as TypeHandle, 1 as TypeHandle];
    let slab = (args.capacity() as u64) * size_of::<TypeHandle>() as u64;
    assert_eq!(node_heap_bytes(&TypeNode::App(name, args)), name_cap + slab);
    assert_eq!(node_heap_bytes(&TypeNode::Arrow(0, 1)), 0);
}
