//! ADR 29.9.26h AC 4, rewritten by ADR 1.10.26g AC 3: a nullary fold is not
//! allocated — it returns one shared read-only cell — and the `MuNullary`
//! class now stamps only a fold whose cell codegen declined.
//!
//! `albite_codegen` has no elaborator, so the fixture is 28.9.26f's list
//! written as the Core it elaborates to: `Lst = μα. () + (Nat × α)`, with
//! `LNil()` as `inl ()` and `LCons(n, t)` as `inr (n, t)`, reached through
//! both heap-fold paths — `compile_fold` and a μ-typed injection.

use super::*;
use crate::codegen::data::mu_types::fold_class;
use crate::codegen::AllocClass;
use inkwell::context::Context;

fn list_mu() -> Type {
    let alpha = Type::TyVar("α_Lst".into());
    let body = Type::Sum(
        Box::new(Type::Unit),
        Box::new(Type::Product(Box::new(Type::Nat), Box::new(alpha))),
    );
    Type::Mu("α_Lst".into(), Box::new(body))
}

fn list_sum() -> Type {
    unwrap_mu_type(&list_mu())
}

/// `fold [Lst] (inl ())` — `LNil()` through `compile_fold`.
fn nil_by_fold() -> Term {
    Term::Fold(
        list_mu(),
        Box::new(Term::Inl(list_sum(), Box::new(Term::Unit))),
    )
}

/// `fold [Lst] (inr (head, tail))` — `LCons(head, tail)` through `compile_fold`.
fn cons_by_fold(head: u64, tail: Term) -> Term {
    let pair = Term::Pair(Box::new(Term::NatLit(head)), Box::new(tail));
    Term::Fold(list_mu(), Box::new(Term::Inr(list_sum(), Box::new(pair))))
}

/// A `CodeGen` positioned in the entry block of a `void probe()`.
fn probe_codegen<'ctx>(context: &'ctx Context, alloc_profile: bool) -> CodeGen<'ctx> {
    let mut codegen = CodeGen::new(context, "fold_class");
    codegen.tracing.alloc_profile = alloc_profile;
    let function =
        codegen
            .module
            .add_function("probe", context.void_type().fn_type(&[], false), None);
    codegen
        .builder
        .position_at_end(context.append_basic_block(function, "entry"));
    codegen.compilation.current_fn = Some(function);
    codegen
}

fn finish(codegen: &CodeGen<'_>) -> String {
    codegen.builder.build_return(None).expect("return");
    codegen.module.print_to_string().to_string()
}

/// Compile `LCons(1, LCons(2, LNil()))` through `compile_fold`, then an
/// `LNil()` and an `LCons(3, LNil())` through μ-typed injections, and return
/// the module's IR.
fn emit_fixture(alloc_profile: bool) -> String {
    let context = Context::create();
    let mut codegen = probe_codegen(&context, alloc_profile);
    let list = cons_by_fold(1, cons_by_fold(2, nil_by_fold()));
    codegen.compile_term(&list).expect("fold chain compiles");
    codegen
        .compile_inl(&list_mu(), &Term::Unit)
        .expect("μ-typed nil injection compiles");
    let tail_pair = Term::Pair(Box::new(Term::NatLit(3)), Box::new(nil_by_fold()));
    codegen
        .compile_inr(&list_mu(), &tail_pair)
        .expect("μ-typed cons injection compiles");
    finish(&codegen)
}

fn alloc_calls_with_class(ir: &str, class: u32) -> usize {
    ir.lines()
        .filter(|line| {
            line.contains("call ptr @__albite_alloc(") && line.contains(&format!(", i32 {class})"))
        })
        .count()
}

fn cell_definitions(ir: &str) -> Vec<&str> {
    ir.lines()
        .filter(|line| line.starts_with("@__tg_nullary."))
        .collect()
}

/// Uses of a cell: every mention that is not its definition.
fn cell_references(ir: &str) -> usize {
    ir.lines()
        .filter(|line| !line.starts_with("@__tg_nullary."))
        .map(|line| line.matches("@__tg_nullary.").count())
        .sum()
}

/// The fixture's allocation calls by class: `(nullary, mu)`.
fn nullary_and_mu_alloc_counts(ir: &str) -> (usize, usize) {
    (
        alloc_calls_with_class(ir, AllocClass::MuNullary as u32),
        alloc_calls_with_class(ir, AllocClass::Mu as u32),
    )
}

#[test]
fn profiled_ir_stamps_exactly_the_nullary_folds() {
    // Nullary folds are not allocated (ADR 1.10.26g), so a profiled build
    // stamps none: every nullary fold of `Lst` became the shared cell, and
    // only the three conses — two folded, one injected — allocate.
    let ir = emit_fixture(true);
    assert_eq!(nullary_and_mu_alloc_counts(&ir), (0, 3), "{ir}");
}

#[test]
fn non_profiled_ir_carries_no_nullary_class() {
    let ir = emit_fixture(false);
    assert_eq!(nullary_and_mu_alloc_counts(&ir), (0, 3), "{ir}");
}

/// ADR 1.10.26g AC 3: the three `LNil`s share one read-only, link-once cell.
#[test]
fn nullary_folds_reference_one_shared_cell() {
    let ir = emit_fixture(false);
    let definitions = cell_definitions(&ir);
    assert_eq!(definitions.len(), 1, "{ir}");
    let definition = definitions[0];
    assert!(
        definition.contains("= linkonce_odr hidden unnamed_addr constant"),
        "{definition}"
    );
    assert!(definition.contains("align 16"), "{definition}");
    // `Lst`'s payload is `Nat × ptr`, 16 bytes; `LNil` is tag 0 alone, so
    // the whole cell is zero.
    assert!(
        definition.starts_with("@__tg_nullary.16.0 "),
        "{definition}"
    );
    assert!(
        definition.contains("constant { i32, [16 x i8] } zeroinitializer"),
        "{definition}"
    );
    // The two `LNil` tails stored into conses. The fixture discards the
    // standalone injected `LNil`, so its use is asserted below by value.
    assert_eq!(cell_references(&ir), 2, "{ir}");
    let context = Context::create();
    let mut codegen = probe_codegen(&context, false);
    let injected = codegen
        .compile_inl(&list_mu(), &Term::Unit)
        .expect("μ-typed nil injection compiles");
    let cell = codegen
        .module
        .get_global("__tg_nullary.16.0")
        .expect("the injection added the cell");
    assert_eq!(injected.into_pointer_value(), cell.as_pointer_value());
    // Three allocation sites remain, the `LCons`s.
    assert_eq!(
        ir.lines()
            .filter(|line| line.contains("call ptr @__albite_alloc("))
            .count(),
        3,
        "{ir}"
    );
}

/// ADR 1.10.26g AC 3: a nullary fold escape analysis marked non-escaping
/// takes the cell too, not a `mu_stack` alloca.
#[test]
fn nullary_cell_precedes_the_stack_fold() {
    let context = Context::create();
    let mut codegen = probe_codegen(&context, false);
    codegen.naming.current_binding_name = Some("local_nil".into());
    codegen.defs.non_escaping_folds.insert("local_nil".into());
    codegen.compile_term(&nil_by_fold()).expect("nil compiles");
    let ir = finish(&codegen);
    assert!(!ir.contains("mu_stack"), "{ir}");
    assert_eq!(cell_definitions(&ir).len(), 1, "{ir}");

    // The same binding with a non-empty fold still stack-folds: the cell
    // path, not the escape-analysis path, changed.
    let context = Context::create();
    let mut codegen = probe_codegen(&context, false);
    codegen.naming.current_binding_name = Some("local_cons".into());
    codegen.defs.non_escaping_folds.insert("local_cons".into());
    codegen
        .compile_term(&cons_by_fold(1, nil_by_fold()))
        .expect("cons compiles");
    let ir = finish(&codegen);
    assert_eq!(ir.matches("%mu_stack = alloca").count(), 1, "{ir}");
    // Its `LNil` tail is still the cell.
    assert_eq!(cell_definitions(&ir).len(), 1, "{ir}");
}

/// ADR 1.10.26g AC 3: an injection that is not μ-typed is a by-value struct,
/// never a cell.
#[test]
fn a_non_mu_injection_gets_no_cell() {
    let context = Context::create();
    let mut codegen = probe_codegen(&context, false);
    let sum = Type::Sum(Box::new(Type::Unit), Box::new(Type::Nat));
    let plain_injection = codegen
        .compile_inl(&sum, &Term::Unit)
        .expect("plain injection compiles");
    assert!(plain_injection.is_struct_value(), "{plain_injection:?}");
    let ir = finish(&codegen);
    assert!(cell_definitions(&ir).is_empty(), "{ir}");
    assert!(!ir.contains("call ptr @__albite_alloc("), "{ir}");
}

/// ADR 1.10.26g AC 3: a nested nullary constructor's cell carries its outer
/// tag in the `i32` and each nested tag at 4-byte strides in the payload —
/// `Leaf(Blue)` of `μα. (() + ()) + (α × α)` is `inl (inr ())`, path `[0, 1]`.
#[test]
fn a_nested_nullary_cell_stores_its_nested_tag_in_the_payload() {
    let alpha = Type::TyVar("α_Pt".into());
    let color = Type::Sum(Box::new(Type::Unit), Box::new(Type::Unit));
    let body = Type::Sum(
        Box::new(color.clone()),
        Box::new(Type::Product(Box::new(alpha.clone()), Box::new(alpha))),
    );
    let painted = Type::Mu("α_Pt".into(), Box::new(body));
    let blue = Term::Inr(color, Box::new(Term::Unit));
    let leaf_blue = Term::Fold(
        painted.clone(),
        Box::new(Term::Inl(unwrap_mu_type(&painted), Box::new(blue))),
    );
    let context = Context::create();
    let mut codegen = probe_codegen(&context, false);
    codegen.compile_term(&leaf_blue).expect("leaf compiles");
    let ir = finish(&codegen);
    let definitions = cell_definitions(&ir);
    assert_eq!(definitions.len(), 1, "{ir}");
    // The test module's layout is the host's; both are little-endian today.
    assert!(
        definitions[0].starts_with(
            "@__tg_nullary.16.0_1 = linkonce_odr hidden unnamed_addr constant \
             { i32, [16 x i8] } { i32 0, [16 x i8] c\"\\01\\00\\00\\00"
        ),
        "{ir}"
    );
}

/// ADR 1.10.26g AC 3: a μ type whose body does not lower to
/// `{ i32, [N x i8] }` — here a bare product — has no cell layout, so its
/// nullary fold goes to the heap and a profiled build stamps it nullary.
/// `Lst` cannot decline, so it cannot show this.
#[test]
fn a_declined_cell_folds_to_the_heap_as_nullary() {
    let alpha = Type::TyVar("α_P".into());
    let product_mu = Type::Mu(
        "α_P".into(),
        Box::new(Type::Product(Box::new(Type::Nat), Box::new(alpha))),
    );
    let sum = Type::Sum(
        Box::new(Type::Unit),
        Box::new(Type::Product(Box::new(Type::Nat), Box::new(Type::Nat))),
    );
    let nullary = Term::Fold(product_mu, Box::new(Term::Inl(sum, Box::new(Term::Unit))));
    let context = Context::create();
    let mut codegen = probe_codegen(&context, true);
    codegen
        .compile_term(&nullary)
        .expect("declined fold compiles");
    let ir = finish(&codegen);
    assert!(cell_definitions(&ir).is_empty(), "{ir}");
    assert_eq!(
        alloc_calls_with_class(&ir, AllocClass::MuNullary as u32),
        1,
        "{ir}"
    );
}

#[test]
fn fold_class_is_nullary_only_when_profiled_and_nullary() {
    assert_eq!(fold_class(true, true), AllocClass::MuNullary);
    assert_eq!(fold_class(true, false), AllocClass::Mu);
    assert_eq!(fold_class(false, true), AllocClass::Mu);
    assert_eq!(fold_class(false, false), AllocClass::Mu);
}
