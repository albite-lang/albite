//! Statement families declare, expand and refuse (ADR 4.10.26i D5, AC 2).
//! Their expression arguments are in `family_arguments.rs` (ADR 6.10.26d).

use super::{def, first_code};
use crate::elaborate::tests::elab_ok;
use crate::elaborate::types::families::expand_simultaneously;
use albite_core::{Term, Type};

pub(super) const LE: &str = "pub type Le(n: Nat, m: Nat): Prop = exists k: Nat. n + k == m\n";

/// `Σ k:Nat. Eq<Nat, a + k, b>` — `Le<a, b>` expanded by hand.
fn le_expanded(a: &str, b: &str) -> Type {
    Type::sigma(
        "k",
        Type::Nat,
        Type::eq(
            Type::Nat,
            Term::nat_add(Term::var(a), Term::var("k")),
            Term::var(b),
        ),
    )
}

/// The statement of a theorem `name(params…) -> S`, under its `Pi`s.
pub(super) fn statement<'t>(ty: &'t Type) -> &'t Type {
    match ty {
        Type::Pi(_, _, body) | Type::Arrow(_, body) => statement(body),
        other => other,
    }
}

// 4.10.26i AC2: `le_refl` states `Le<n, n>`, which the kernel sees expanded.
#[test]
fn le_refl_states_the_expanded_sigma() {
    let source = format!("{LE}pub theorem le_refl(n: Nat) -> Le<n, n> {{ (0, refl) }}");
    let defs = elab_ok(&source);
    assert_eq!(statement(&def(&defs, "le_refl").ty), &le_expanded("n", "n"));
}

// 4.10.26i AC2 (D5): `Le<m, n>` against `Le(n, m)` swaps, never collapses.
#[test]
fn swapped_arguments_expand_simultaneously() {
    let source = format!("{LE}pub theorem keep(n: Nat, m: Nat, h: Le<m, n>) -> Le<m, n> {{ h }}");
    let defs = elab_ok(&source);
    assert_eq!(statement(&def(&defs, "keep").ty), &le_expanded("m", "n"));
    // And the pure expansion, with the arguments exactly swapped.
    let params = [("n".to_string(), Type::Nat), ("m".to_string(), Type::Nat)];
    let body = le_expanded("n", "m");
    let swapped = expand_simultaneously(&params, &body, &[Term::var("m"), Term::var("n")]);
    assert_eq!(swapped, le_expanded("m", "n"));
    let same = expand_simultaneously(&params, &body, &[Term::var("n"), Term::var("m")]);
    assert_eq!(same, body);
}

// 4.10.26i AC2 (D5): a family declared in one module expands in another.
#[test]
fn a_family_imported_from_another_module_expands() {
    use crate::driver::{run_file, Mode, PipelineResult};
    let dir = tempfile::TempDir::new().expect("a temp dir");
    let main = "mod order;\nuse order::Le;\n\
                pub theorem le_refl_there(n: Nat) -> Le<n, n> { (0, refl) }\n";
    std::fs::write(dir.path().join("main.tg"), main).expect("main.tg");
    std::fs::write(dir.path().join("order.tg"), LE).expect("order.tg");
    let result = run_file(&dir.path().join("main.tg"), Mode::Check, false);
    assert!(
        matches!(result, Ok(PipelineResult::Checked { .. })),
        "{result:?}"
    );
}

// 4.10.26i AC2 (D5): a parameter ranging over a proposition is E0067.
#[test]
fn a_parameter_that_sorts_prop_is_refused() {
    assert_eq!(first_code("type Bad(h: 0 == 0): Prop = Void"), "E0067");
}

// 4.10.26i AC2 (D5): type parameters on a family are not yet supported.
#[test]
fn a_family_with_type_parameters_is_refused() {
    assert_eq!(first_code("type F<T>(x: Nat): Prop = Void"), "E0100");
}

// 4.10.26i AC2 (D5): `: Type` with parameters is a value-indexed data type.
#[test]
fn a_data_family_is_not_yet_supported() {
    assert_eq!(first_code("type Vec(n: Nat): Type = Nat"), "E0100");
}

// 4.10.26i AC2 (D5), re-pointed by 6.10.26d AC3 (D6): a type written as a
// family argument is a type where a term belongs, E0023 — no longer E0100.
#[test]
fn a_type_argument_to_a_family_is_refused_as_a_type() {
    for argument in ["Nat", "(Nat, Nat)", "Box<Nat>"] {
        let source = format!(
            "type Box<T> = MkBox(T)\n{LE}pub theorem g(n: Nat) -> Le<{argument}, n> {{ sorry }}"
        );
        assert_eq!(first_code(&source), "E0023", "{argument}");
    }
}

// 4.10.26i AC2 (D5): an argument of the wrong type is a mismatch.
#[test]
fn an_argument_of_the_wrong_type_is_a_mismatch() {
    let source = format!("{LE}pub theorem g(b: Bool) -> Le<b, b> {{ (0, refl) }}");
    assert_eq!(first_code(&source), "E0010");
}

// 4.10.26i AC2 (D5): arity is checked, and a bare use has the wrong arity.
#[test]
fn a_family_use_checks_its_arity() {
    let one = format!("{LE}pub theorem g(n: Nat) -> Le<n> {{ (0, refl) }}");
    let bare = format!("{LE}pub theorem g(n: Nat) -> Le {{ (0, refl) }}");
    let arity = first_code(&one);
    assert_eq!(first_code(&bare), arity);
    assert_ne!(arity, "E0100");
}
