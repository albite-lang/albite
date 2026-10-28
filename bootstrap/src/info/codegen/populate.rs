//! Populating a `CodeGen` from elaborated definitions, for `info codegen
//! symbols` (moved here from `info/commands/mod.rs` by ADR 30.9.26d).
//!
//! Codegen-gated code lives under `info/codegen/`, the one `info` directory
//! the `--no-default-features` mutation sweep excludes, because no test in
//! that build can reach it.

use albite_bootstrap::driver::CoreDef;
use albite_codegen::CodeGen;
use albite_core::types::Type;

/// Map a definition name to its LLVM-level name.
fn def_llvm_name(name: &str) -> String {
    if name == "main" {
        "albite_main".to_string()
    } else {
        name.to_string()
    }
}

/// Declare functions, register term definitions, and compile all defs into codegen.
pub(crate) fn populate_codegen_defs(codegen: &mut CodeGen, defs: &[CoreDef], verbose: bool) {
    // `info`'s own compile entry erases as `compile` does: dependency (ADR
    // 30.9.26d D6), then proofs (ADR 3.10.26e D4).
    let erased: Vec<_> = defs.iter().map(CoreDef::erase_for_codegen).collect();
    let defs = erased.as_slice();
    // Pass 1: Declare all functions
    for def in defs {
        let llvm_name = def_llvm_name(&def.name);
        if matches!(&def.ty, Type::Forall(_, _)) {
            codegen.register_def_type(&llvm_name, &def.ty);
        } else if let Err(e) = codegen.declare_def(&llvm_name, &def.ty) {
            eprintln!("warning: declaration failed for '{}': {}", def.name, e);
        }
    }

    // Pass 2: Register term definitions for monomorphization
    for def in defs {
        let llvm_name = def_llvm_name(&def.name);
        codegen.register_term_def(&llvm_name, def.term.term.clone());
    }

    // Pass 3: Compile all definitions to build the symbol map
    for def in defs {
        let llvm_name = def_llvm_name(&def.name);
        if matches!(&def.ty, Type::Forall(_, _)) {
            continue;
        }
        if let Err(e) = codegen.compile_def_with_span(
            &llvm_name,
            &def.term.term,
            &def.ty,
            def.term.span.map(|s| s.start),
        ) {
            if verbose {
                eprintln!("warning: codegen failed for '{}': {}", def.name, e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use albite_bootstrap::driver::CoreDef;
    use albite_bootstrap::span::Span;
    use albite_codegen::inkwell::context::Context as LlvmContext;
    use albite_codegen::CodeGen;
    use albite_core::terms::SpannedTerm;
    use albite_core::{Term, Type};

    use super::populate_codegen_defs;

    // 30.9.26d AC8: `info codegen symbols`' entry declares a dependent lemma
    // under its erased arrow, so `declare_def`'s debug assertion holds.
    #[test]
    fn populate_declares_a_dependent_definition_as_its_arrow() {
        let body = Type::eq(Type::Nat, Term::var("n"), Term::var("n"));
        let lemma = CoreDef {
            name: "lemma".to_string(),
            ty: Type::pi("n", Type::Nat, body.clone()),
            term: SpannedTerm::generated(Term::lambda(
                "n",
                Type::Nat,
                Term::refl(Type::Nat, Term::var("n")),
            )),
            span: Span::new(0, 1),
        };
        let llvm_context = LlvmContext::create();
        let mut codegen = CodeGen::new(&llvm_context, "populate_tests");
        populate_codegen_defs(&mut codegen, &[lemma], false);
        assert_eq!(
            codegen.get_def_type("lemma"),
            Some(Type::arrow(Type::Nat, body))
        );
    }

    // 3.10.26e AC 4: a `natind` proof body is erased before codegen, so it
    // compiles — the PHI failure 30.9.26d parked would make this an error.
    #[test]
    fn populate_erases_proofs() {
        let body = Type::eq(Type::Nat, Term::var("i"), Term::var("i"));
        let step = Term::lambda(
            "j",
            Type::Nat,
            Term::lambda("ih", body.clone(), Term::var("ih")),
        );
        let proof = Term::natind(
            "i",
            Type::arrow(Type::Nat, body),
            Term::refl(Type::Nat, Term::Zero),
            step,
            Term::var("n"),
        );
        let lemma = CoreDef {
            name: "lemma".to_string(),
            ty: Type::arrow(
                Type::Nat,
                Type::eq(Type::Nat, Term::var("n"), Term::var("n")),
            ),
            term: SpannedTerm::generated(Term::lambda("n", Type::Nat, proof)),
            span: Span::new(0, 1),
        };
        let llvm_context = LlvmContext::create();
        let mut codegen = CodeGen::new(&llvm_context, "populate_tests");
        populate_codegen_defs(&mut codegen, &[lemma], true);
        assert!(codegen.module().verify().is_ok());
        let ir = codegen.module().print_to_string().to_string();
        assert!(!ir.contains("natrec_header"), "{ir}");
    }
}
