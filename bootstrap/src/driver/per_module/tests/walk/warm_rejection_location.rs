//! Termination rejections through the driver, cold then warm (ADR 3.10.26r
//! AC 3a).
//!
//! The unit tests beside the gate adapter check each half — the file lookup,
//! the carried sites, the merge — over hand-built inputs. This runs §1's
//! fixtures through the real walk and the real elaboration cache twice, and
//! asks what a reader would be shown: the record the diagnostic renderer
//! builds, resolved against the file the error names, exactly as
//! `check --json` resolves it.

use std::path::Path;

use super::elaborate_tree_with_cache;
use crate::driver::diagnostics::json::{elab_record, Level, ResolvedSource};
use crate::elaborate::ElabError;

const MAIN: &str = "pub mod a;\npub mod b;\n\nfn main() -> Nat { 0 }\n";

/// Where the renderer would put `error`: its file (the entry's when it names
/// none, as `get_error_source` falls back) and the 1-based line of its span.
fn rendered_location(dir: &Path, error: &ElabError) -> (String, Option<u32>) {
    let file = error
        .file_path
        .clone()
        .unwrap_or_else(|| dir.join("main.tg"));
    let source = std::fs::read_to_string(&file).unwrap();
    let filename = file.display().to_string();
    let resolved = ResolvedSource {
        source: &source,
        filename: &filename,
    };
    let record = elab_record(error, Level::Error, &resolved, None);
    let span = record.primary_span.expect("an E0063 has a primary span");
    let name = Path::new(&span.location.file)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    (name, span.location.line_start)
}

/// Elaborate the three-module project cold, then warm, and return what each
/// run's rejections render as.
fn cold_then_warm(a_src: &str, b_src: &str) -> [Vec<(String, Option<u32>)>; 2] {
    let dir = tempfile::tempdir().unwrap();
    let files = [("main.tg", MAIN), ("a.tg", a_src), ("b.tg", b_src)];
    [(); 2].map(|()| {
        let errors = elaborate_tree_with_cache(dir.path(), &files).unwrap_err();
        errors
            .iter()
            .filter(|error| error.kind.code() == "E0063")
            .map(|error| rendered_location(dir.path(), error))
            .collect()
    })
}

/// The axiom sits on `b.tg`'s line 3 in every fixture below.
fn at_the_proof() -> Vec<(String, Option<u32>)> {
    vec![("b.tg".to_string(), Some(3))]
}

const PARTIAL_SEED: &str = "#[partial]\npub fn h(n: Nat) -> Nat { h(n) }\n";
const FOREIGN_SEED: &str =
    "extern \"C\" fn tg_type_nat() -> Nat\npub fn h(n: Nat) -> Nat { tg_type_nat() }\n";
const AXIOM_OVER_A: &str = "use main::a::h;\n\npub axiom ax(): Eq<Nat, h(0), h(0)>\n";

#[test]
fn a_separate_module_partial_rejection_renders_at_the_proof_cold_and_warm() {
    for run in cold_then_warm(PARTIAL_SEED, AXIOM_OVER_A) {
        assert_eq!(run, at_the_proof());
    }
}

#[test]
fn a_separate_module_foreign_rejection_renders_at_the_proof_cold_and_warm() {
    for run in cold_then_warm(FOREIGN_SEED, AXIOM_OVER_A) {
        assert_eq!(run, at_the_proof());
    }
}

#[test]
fn a_same_module_partial_rejection_renders_once_at_the_proof_cold_and_warm() {
    let proof_and_seed =
        "#[partial]\npub fn h(n: Nat) -> Nat { h(n) }\npub axiom ax(): Eq<Nat, h(0), h(0)>\n";
    for run in cold_then_warm("pub fn unused() -> Nat { 0 }\n", proof_and_seed) {
        assert_eq!(run, at_the_proof());
    }
}

#[test]
fn a_same_module_foreign_rejection_renders_once_at_the_proof_cold_and_warm() {
    let proof_and_seed = "extern \"C\" fn tg_type_nat() -> Nat\n\npub axiom ax(): Eq<Nat, tg_type_nat(), tg_type_nat()>\n";
    for run in cold_then_warm("pub fn unused() -> Nat { 0 }\n", proof_and_seed) {
        assert_eq!(run, at_the_proof());
    }
}
