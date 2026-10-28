//! Tests for `region { e }` elaboration (ADR 9.10.26o AC 3): the accepted
//! shapes, E0094 on the value and on a crossing exit, E0095's type-variable
//! half and E0096, plus the first-order allow-list itself.

use albite_core::{Term, Type};

use super::first_order_violation;
use crate::elaborate::error::{ElabErrorKind, RegionErrorKind};
use crate::elaborate::tests::{elab_err, elab_ok};

fn region_kinds(source: &str) -> Vec<RegionErrorKind> {
    elab_err(source)
        .into_iter()
        .filter_map(|e| match e.kind {
            ElabErrorKind::Region(kind) => Some(kind),
            _ => None,
        })
        .collect()
}

fn codes(source: &str) -> Vec<&'static str> {
    region_kinds(source)
        .iter()
        .map(RegionErrorKind::code)
        .collect()
}

// 9.10.26o AC3: a Nat-valued region is accepted and elaborates to Term::Region.
#[test]
fn a_nat_valued_region_elaborates_to_a_region_term() {
    let defs = elab_ok("fn f(n: Nat) -> Nat { region { n + 1 } }");
    assert!(defs[0].term.contains_region(), "{:?}", defs[0].term);
}

// 9.10.26o AC3: first-order data is accepted — a string, a tuple, an ADT.
#[test]
fn first_order_values_are_accepted() {
    elab_ok("fn f() -> String { region { \"x\" } }");
    elab_ok("fn f() -> (Nat, Bool) { region { (1, true) } }");
    elab_ok(
        "type Pair2 = MkPair2(Nat, String)\n\
         fn f() -> Pair2 { region { MkPair2(1, \"a\") } }",
    );
}

// 9.10.26o AC3: E0094 for a closure-valued region.
#[test]
fn a_closure_valued_region_is_e0094() {
    assert_eq!(
        codes("fn f() -> Nat { let g = region { |x: Nat| x }; g(1) }"),
        vec!["E0094"]
    );
}

// 9.10.26o AC3: a `return` of a function type crossing a region is E0094.
#[test]
fn a_return_of_a_function_type_crossing_a_region_is_e0094() {
    let kinds = region_kinds(
        "fn f(b: Bool) -> Nat -> Nat {\n\
             let n: Nat = region { if b { 1 } else { return |x: Nat| x } };\n\
             |y: Nat| y + n\n\
         }",
    );
    assert!(
        matches!(kinds.as_slice(), [RegionErrorKind::NotFirstOrder { .. }]),
        "{kinds:?}"
    );
}

// 9.10.26o AC3: a `return` inside a lambda inside a region exits the lambda,
// not the region, so E0094 ignores it.
#[test]
fn a_return_inside_a_lambda_inside_a_region_is_ignored() {
    elab_ok(
        "fn apply(g: Nat -> Nat, n: Nat) -> Nat { g(n) }\n\
         fn f() -> Nat -> Nat {\n\
             let n = region { apply(|x: Nat| { return x }, 1) };\n\
             |y: Nat| y + n\n\
         }",
    );
}

// 9.10.26o AC3: E0095's type-variable half — an outer variable of a type
// parameter's type is refused; a polymorphic top-level function is not.
#[test]
fn an_outer_type_variable_binding_is_e0095() {
    let kinds = region_kinds("fn f<T>(x: T) -> Nat { region { let y = x; 1 } }");
    assert_eq!(
        kinds,
        vec![RegionErrorKind::ReachesOutside {
            name: "x".into(),
            reason: "whose type holds a type parameter".into(),
        }]
    );
    elab_ok(
        "fn id<T>(x: T) -> T { x }\n\
         fn f() -> Nat { region { id(1) } }",
    );
}

// 9.10.26o AC3: E0096 — a region in a theorem body.
#[test]
fn a_region_in_a_theorem_is_e0096() {
    assert_eq!(
        codes("theorem t(): Eq<Nat, 1, 1> { region { refl } }"),
        vec!["E0096"]
    );
}

// 9.10.26o D4: the allow-list is closed — every other kind is refused, a
// μ-bound variable is not free, and poison raises nothing.
#[test]
fn the_first_order_allow_list_is_closed() {
    for accepted in [
        Type::Nat,
        Type::Int,
        Type::Bool,
        Type::String,
        Type::Unit,
        Type::Error,
        Type::Product(Box::new(Type::Nat), Box::new(Type::String)),
        Type::Mu(
            "a".into(),
            Box::new(Type::Sum(
                Box::new(Type::Unit),
                Box::new(Type::Product(
                    Box::new(Type::Nat),
                    Box::new(Type::TyVar("a".into())),
                )),
            )),
        ),
        Type::Adt(
            "W".into(),
            vec![],
            vec![("MkW".into(), Type::Nat), ("Other".into(), Type::Unit)],
        ),
    ] {
        assert_eq!(first_order_violation(&accepted), None, "{accepted:?}");
    }
    let arrow = Type::Arrow(Box::new(Type::Nat), Box::new(Type::Nat));
    for refused in [
        arrow.clone(),
        Type::Void,
        Type::TyVar("T".into()),
        Type::Ref(Box::new(Type::Nat)),
        Type::Ptr(Box::new(Type::Nat)),
        Type::Forall("T".into(), Box::new(Type::Nat)),
        Type::Eq(
            Box::new(Type::Nat),
            Box::new(Term::Zero),
            Box::new(Term::Zero),
        ),
    ] {
        assert_eq!(first_order_violation(&refused), Some(refused.clone()));
    }
    let nested = Type::Adt("Box".into(), vec![], vec![("MkBox".into(), arrow.clone())]);
    assert_eq!(first_order_violation(&nested), Some(arrow.clone()));
    let applied = Type::App("Option".into(), vec![arrow.clone()]);
    assert_eq!(first_order_violation(&applied), Some(arrow));
}

// 9.10.26o D3: `find_return_in_expr` descends into a region, so a `return`
// inside a region inside a `try` stays E0044.
#[test]
fn a_return_in_a_region_in_a_try_is_still_e0044() {
    let errors = elab_err(
        "type Result<T, E> = Ok(T) | Err(E)\n\
         fn f() -> Result<Nat, String> { try { region { return Ok(1) } } }",
    );
    assert!(
        errors
            .iter()
            .any(|e| matches!(e.kind, ElabErrorKind::ReturnInsideTryBlock)),
        "{errors:?}"
    );
}

// 9.10.26o AC6: the marker reads the region keyword from module text, so a
// warm run with every module cache-served still sets it; a `region` in a
// string, a comment or an identifier part is not one.
#[test]
fn the_region_marker_reads_the_keyword_from_source() {
    use crate::driver::modules::{source_uses_regions, SourceMap};
    assert!(source_uses_regions("fn f() -> Nat { region { 1 } }"));
    assert!(!source_uses_regions("fn f() -> Nat { 1 }"));
    assert!(!source_uses_regions(
        "// region\nfn regions() -> String { \"region\" }"
    ));
    let mut map = SourceMap::new();
    map.insert("a.tg".into(), "fn a() -> Nat { 1 }".into());
    assert!(!map.uses_regions());
    map.insert("b.tg".into(), "fn b() -> Nat { region { 2 } }".into());
    assert!(map.uses_regions());
}

/// The types of the first `Term::Region` in `term`, depth first.
fn first_region_types(term: &Term) -> Option<albite_core::terms::RegionTypes> {
    if let Term::Region(types, _) = term {
        return Some(types.clone());
    }
    let mut found = None;
    term.for_each_subterm(|sub| {
        if found.is_none() {
            found = first_region_types(sub);
        }
    });
    found
}

// 13.10.26q AC3 (D2): a region records its value type, and no exit type when
// nothing in its body crosses it.
#[test]
fn a_region_records_its_value_type() {
    let defs = elab_ok("fn f() -> String { region { \"a\" ++ \"b\" } }");
    let types = first_region_types(&defs[0].term).expect("a region");
    assert_eq!(types.value, Type::String);
    assert_eq!(types.exit, None);
}

// 13.10.26q AC3 (D2): a crossing `return` records the innermost return type
// as the region's exit type, beside its value type.
#[test]
fn a_crossing_return_records_the_exit_type() {
    let defs = elab_ok(
        "fn f(n: Nat) -> String {\n\
             let k: Nat = region { if n > 1 { return \"big\" } else { n } };\n\
             \"small\"\n\
         }",
    );
    let types = first_region_types(&defs[0].term).expect("a region");
    assert_eq!(types.value, Type::Nat);
    assert_eq!(types.exit, Some(Type::String));
}

// 13.10.26q AC3 (D2): a region inside a `try` records the `try`'s type as its
// exit, not the enclosing function's — the closure a crossing `?` leaves.
#[test]
fn a_region_inside_a_try_records_the_trys_type() {
    let defs = elab_ok(
        "type Result<T, E> = Ok(T) | Err(E)\n\
         fn half(n: Nat) -> Result<Nat, String> { if n == 0 { Err(\"z\") } else { Ok(n) } }\n\
         fn f(n: Nat) -> Nat {\n\
             let r: Result<Nat, String> = try { region { half(n)? + 1 } };\n\
             match r { Ok(m) => m, Err(_) => 0 }\n\
         }",
    );
    let f = defs.iter().find(|def| def.name == "f").expect("f");
    let types = first_region_types(&f.term).expect("a region");
    assert_eq!(types.value, Type::Nat);
    let exit = types.exit.expect("the `?` crosses the region");
    assert_ne!(exit, Type::Nat, "the try's Result, not f's Nat: {exit}");
    assert!(
        matches!(exit, Type::Sum(..) | Type::Mu(..) | Type::Adt(..)),
        "{exit}"
    );
}
