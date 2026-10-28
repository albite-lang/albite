//! Constructor ownership: every constructor a pattern names belongs to the
//! type the pattern is over (ADR 4.10.26d D1).
//!
//! A constructor is trusted by its **index**, so a constructor of another type
//! fills that index of the scrutinee with its own payload: `PickWrong(k)` over
//! an `OptWrong` once bound `k : Nat` to the `Unit` at `OptWrong`'s index 0.
//! Two checks close that, both pure:
//!
//! - **the arms agree** — nominal, because both sides are resolved names
//!   ([`owner_agrees`]);
//! - **the scrutinee agrees** — structural, because a binary sum reaches the
//!   match as its encoding and carries no name ([`scrutinee_admits`]). Two
//!   types with one encoding stay interchangeable, exactly as a typed `let`
//!   treats them (P0.4).
//!
//! The structural check compares each declared payload with the scrutinee's
//! payload at the same index, **up to the forms it cannot read**: a type
//! parameter, a recursive reference, a deferred application or poison, on
//! either side, matches anything. So a generic instance, a μ-encoded type and
//! a mutual-recursion marker are admitted, and the refusal is reserved for a
//! payload whose shape provably differs (D7's narrowing).

use albite_core::Type;

use crate::elaborate::env as elab_env;

/// Whether an arm's resolved owner is the type the match was resolved to.
pub(in crate::elaborate) fn owner_agrees(context_owner: &str, arm_owner: &str) -> bool {
    context_owner == arm_owner
}

/// How a normalised scrutinee type splits into one payload per constructor.
#[derive(Debug, PartialEq)]
pub(in crate::elaborate) enum ScrutineeSplit<'t> {
    /// One payload per constructor, in index order.
    Payloads(Vec<&'t Type>),
    /// A form the check cannot read — a type variable, a deferred
    /// application or poison — admitted unread (D7).
    Opaque,
    /// Provably not a sum of that many constructors.
    Mismatch,
}

/// Split `ty` into `count` payloads the way the match walks it: a flat ADT by
/// its variants, a binary sum by its right-nested spine, `μ` unwrapped at
/// each step, and a single constructor as the whole type.
pub(in crate::elaborate) fn split_scrutinee(ty: &Type, count: usize) -> ScrutineeSplit<'_> {
    let mut current = unwrap_mu(ty);
    if is_opaque(current) {
        return ScrutineeSplit::Opaque;
    }
    if count <= 1 {
        return ScrutineeSplit::Payloads(vec![current]);
    }
    if let Type::Adt(_, _, variants) = current {
        return if variants.len() == count {
            ScrutineeSplit::Payloads(variants.iter().map(|(_, payload)| payload).collect())
        } else {
            ScrutineeSplit::Mismatch
        };
    }
    let mut payloads = Vec::with_capacity(count);
    for _ in 1..count {
        match current {
            Type::Sum(left, right) => {
                payloads.push(left.as_ref());
                current = unwrap_mu(right);
            }
            other if is_opaque(other) => return ScrutineeSplit::Opaque,
            _ => return ScrutineeSplit::Mismatch,
        }
    }
    payloads.push(current);
    ScrutineeSplit::Payloads(payloads)
}

/// Whether a normalised scrutinee type is `constructors`' encoding at some
/// instantiation of their type's parameters — D1(b).
pub(in crate::elaborate) fn scrutinee_admits(
    constructors: &[elab_env::Constructor],
    ty: &Type,
) -> bool {
    match split_scrutinee(ty, constructors.len()) {
        ScrutineeSplit::Opaque => true,
        ScrutineeSplit::Mismatch => false,
        ScrutineeSplit::Payloads(payloads) => constructors
            .iter()
            .zip(payloads)
            .all(|(ctor, payload)| payload_admits(&ctor.fields, payload)),
    }
}

/// `scrutinee_admits` without the opaque admission: the scrutinee is read and
/// IS `constructors`' encoding. A bare name spelled like a constructor with
/// fields is refused only on this evidence (ADR 6.10.26k D2 c); over a type
/// variable it stays the binder it is today.
pub(in crate::elaborate) fn scrutinee_owned_by(
    constructors: &[elab_env::Constructor],
    ty: &Type,
) -> bool {
    match split_scrutinee(ty, constructors.len()) {
        ScrutineeSplit::Payloads(_) => scrutinee_admits(constructors, ty),
        ScrutineeSplit::Opaque | ScrutineeSplit::Mismatch => false,
    }
}

/// Whether `payload` can carry a constructor with these declared fields:
/// `()` for none, the field for one, right-nested pairs for more.
pub(in crate::elaborate) fn payload_admits(fields: &[Type], payload: &Type) -> bool {
    let mut declared = fields.iter().rev();
    let Some(last) = declared.next() else {
        return shape_admits(&Type::Unit, payload);
    };
    let expected = declared.fold(last.clone(), |rest, field| {
        Type::product(field.clone(), rest)
    });
    shape_admits(&expected, payload)
}

/// Whether `actual` has the shape `declared` describes, reading neither
/// side's unreadable forms. A declared `TyVar` is a type parameter or a named
/// (possibly recursive) reference; a declared `Mu`/`Adt` is a whole type the
/// payload may carry encoded or by name.
pub(in crate::elaborate) fn shape_admits(declared: &Type, actual: &Type) -> bool {
    #[allow(clippy::enum_glob_use)] // Reason: a table over every former
    use Type::*;
    match (declared, actual) {
        (TyVar(_) | App(..) | Error | Mu(..) | Adt(..) | Forall(..) | Pi(..) | Eq(..), _) => true,
        (_, TyVar(_) | App(..) | Error) => true,
        (Bool, Bool) | (Nat, Nat) | (Int, Int) | (Unit, Unit) | (Void, Void) => true,
        (String, String) => true,
        (Product(a, b), Product(c, d)) | (Sum(a, b), Sum(c, d)) | (Arrow(a, b), Arrow(c, d)) => {
            shape_admits(a, c) && shape_admits(b, d)
        }
        (Ptr(a), Ptr(b)) | (Ref(a), Ref(b)) => shape_admits(a, b),
        _ => false,
    }
}

fn unwrap_mu(ty: &Type) -> &Type {
    match ty {
        Type::Mu(_, body) => unwrap_mu(body),
        other => other,
    }
}

fn is_opaque(ty: &Type) -> bool {
    matches!(ty, Type::TyVar(_) | Type::App(..) | Type::Error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elaborate::env::Constructor;

    fn sum(left: Type, right: Type) -> Type {
        Type::Sum(Box::new(left), Box::new(right))
    }

    fn tyvar(name: &str) -> Type {
        Type::TyVar(name.to_string())
    }

    /// `OptWrong = NadaWrong | AlgoWrong(Nat)`.
    fn opt_wrong() -> Vec<Constructor> {
        vec![
            Constructor::test_stub("NadaWrong", 0),
            Constructor::test_with_fields("AlgoWrong", 1, vec![Type::Nat]),
        ]
    }

    /// `OtherWrong = PickWrong(Nat) | SkipWrong`.
    fn other_wrong() -> Vec<Constructor> {
        vec![
            Constructor::test_with_fields("PickWrong", 0, vec![Type::Nat]),
            Constructor::test_stub("SkipWrong", 1),
        ]
    }

    // 4.10.26d AC1: the two pure checks over hand-built types.

    #[test]
    fn owners_agree_only_on_the_same_name() {
        assert!(owner_agrees("OptWrong", "OptWrong"));
        assert!(!owner_agrees("OptWrong", "OtherWrong"));
    }

    #[test]
    fn a_type_admits_its_own_encoding() {
        let opt_encoding = sum(Type::Unit, Type::Nat);
        assert!(scrutinee_admits(&opt_wrong(), &opt_encoding));
    }

    #[test]
    fn another_types_encoding_is_refused() {
        let opt_encoding = sum(Type::Unit, Type::Nat);
        assert!(!scrutinee_admits(&other_wrong(), &opt_encoding));
    }

    #[test]
    fn a_non_sum_scrutinee_is_refused() {
        assert!(!scrutinee_admits(&opt_wrong(), &Type::Nat));
    }

    #[test]
    fn opaque_scrutinees_are_admitted_unread() {
        for ty in [
            tyvar("α_Beta"),
            Type::App("List".into(), vec![]),
            Type::Error,
        ] {
            assert_eq!(split_scrutinee(&ty, 2), ScrutineeSplit::Opaque);
            assert!(scrutinee_admits(&other_wrong(), &ty));
        }
    }

    #[test]
    fn a_generic_instance_is_admitted() {
        let maybe = vec![
            Constructor::test_stub("Nothing", 0),
            Constructor::test_with_fields("Just", 1, vec![tyvar("T")]),
        ];
        let at_string = sum(Type::Unit, Type::String);
        assert!(scrutinee_admits(&maybe, &at_string));
    }

    #[test]
    fn a_recursive_encoding_is_unwrapped() {
        // `Lst = Nil | Cons(Nat, Lst)` ≡ μα. Unit + (Nat × α)
        let lst = vec![
            Constructor::test_stub("Nil", 0),
            Constructor::test_with_fields("Cons", 1, vec![Type::Nat, tyvar("Lst")]),
        ];
        let body = sum(Type::Unit, Type::product(Type::Nat, tyvar("α_Lst")));
        let mu = Type::Mu("α_Lst".to_string(), Box::new(body));
        assert!(scrutinee_admits(&lst, &mu));
        let wrong_head = Type::product(Type::Bool, tyvar("α_Lst"));
        assert!(!scrutinee_admits(&lst, &sum(Type::Unit, wrong_head)));
    }

    #[test]
    fn a_flat_adt_splits_by_its_variants() {
        let tri = vec![
            Constructor::test_stub("Lo", 0),
            Constructor::test_with_fields("Mid", 1, vec![Type::Nat]),
            Constructor::test_stub("Hi", 2),
        ];
        let variants = vec![
            ("Lo".to_string(), Type::Unit),
            ("Mid".to_string(), Type::Nat),
            ("Hi".to_string(), Type::Unit),
        ];
        let flat = Type::Adt("Tri".to_string(), vec![], variants.clone());
        assert!(scrutinee_admits(&tri, &flat));
        let pair = Type::Adt("Pair".to_string(), vec![], variants[..2].to_vec());
        assert!(!scrutinee_admits(&tri, &pair));
        assert!(!scrutinee_admits(&opt_wrong(), &flat));
    }

    #[test]
    fn a_three_constructor_context_refuses_a_binary_sum() {
        let three = vec![
            Constructor::test_with_fields("TA", 0, vec![Type::Nat]),
            Constructor::test_with_fields("TB", 1, vec![Type::Nat]),
            Constructor::test_with_fields("TC", 2, vec![Type::Nat]),
        ];
        assert_eq!(
            split_scrutinee(&sum(Type::Unit, Type::Nat), 3),
            ScrutineeSplit::Mismatch
        );
        assert!(!scrutinee_admits(&three, &sum(Type::Unit, Type::Nat)));
        let nested = sum(Type::Nat, sum(Type::Nat, Type::Nat));
        assert!(scrutinee_admits(&three, &nested));
        let marker_tail = sum(Type::Nat, tyvar("α_Rest"));
        assert_eq!(split_scrutinee(&marker_tail, 3), ScrutineeSplit::Opaque);
    }

    #[test]
    fn a_single_constructor_is_its_whole_payload() {
        let wrap = vec![Constructor::test_with_fields("W", 0, vec![Type::Nat])];
        assert!(scrutinee_admits(&wrap, &Type::Nat));
        assert!(!scrutinee_admits(&wrap, &Type::Bool));
    }

    #[test]
    fn multi_field_payloads_are_right_nested_pairs() {
        let fields = [Type::Nat, Type::Bool, Type::String];
        let nested = Type::product(Type::Nat, Type::product(Type::Bool, Type::String));
        assert!(payload_admits(&fields, &nested));
        let left_nested = Type::product(Type::product(Type::Nat, Type::Bool), Type::String);
        assert!(!payload_admits(&fields, &left_nested));
        assert!(payload_admits(&[], &Type::Unit));
        assert!(!payload_admits(&[], &Type::Nat));
    }

    #[test]
    fn shapes_compare_leaf_by_leaf() {
        let leaves = [
            Type::Bool,
            Type::Nat,
            Type::Int,
            Type::Unit,
            Type::Void,
            Type::String,
        ];
        for (i, a) in leaves.iter().enumerate() {
            for (j, b) in leaves.iter().enumerate() {
                assert_eq!(shape_admits(a, b), i == j, "{a} vs {b}");
            }
        }
        let arrow = |a: Type, b: Type| Type::Arrow(Box::new(a), Box::new(b));
        assert!(shape_admits(
            &arrow(Type::Nat, Type::Bool),
            &arrow(Type::Nat, Type::Bool)
        ));
        assert!(!shape_admits(
            &arrow(Type::Nat, Type::Bool),
            &arrow(Type::Nat, Type::Nat)
        ));
        assert!(!shape_admits(
            &arrow(Type::Nat, Type::Bool),
            &arrow(Type::Bool, Type::Bool)
        ));
        assert!(shape_admits(
            &sum(Type::Nat, Type::Unit),
            &sum(Type::Nat, Type::Unit)
        ));
        assert!(!shape_admits(
            &sum(Type::Nat, Type::Unit),
            &sum(Type::Unit, Type::Unit)
        ));
        let ptr = |t: Type| Type::Ptr(Box::new(t));
        let reference = |t: Type| Type::Ref(Box::new(t));
        assert!(shape_admits(&ptr(Type::Nat), &ptr(Type::Nat)));
        assert!(!shape_admits(&ptr(Type::Nat), &ptr(Type::Bool)));
        assert!(shape_admits(&reference(Type::Nat), &reference(Type::Nat)));
        assert!(!shape_admits(&reference(Type::Nat), &ptr(Type::Nat)));
        assert!(!shape_admits(&Type::Nat, &sum(Type::Nat, Type::Nat)));
    }

    #[test]
    fn unreadable_forms_match_anything() {
        assert!(shape_admits(&tyvar("T"), &Type::Nat));
        assert!(shape_admits(&Type::Nat, &tyvar("α_List")));
        assert!(shape_admits(&Type::Nat, &Type::Error));
        assert!(shape_admits(
            &Type::Nat,
            &Type::App("Forest".into(), vec![])
        ));
        let mu = Type::Mu("α".into(), Box::new(Type::Unit));
        assert!(shape_admits(&mu, &Type::Nat));
        assert!(!shape_admits(&Type::Nat, &mu));
    }
}
