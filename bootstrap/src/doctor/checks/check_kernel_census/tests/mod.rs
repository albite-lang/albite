//! The kernel census (ADR 30.9.26f): ACs 1–3 in memory, AC 4's exit and
//! render half, AC 5 over one elaborated golden. The helpers every sibling
//! shares live here; each sibling holds one AC's tests.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use albite_core::context::GlobalAdmission;
use albite_core::terms::termination::AdmissionState;
use albite_core::terms::{SpannedTerm, TermSpan};
use albite_core::typecheck::check_proof;
use albite_core::{Context, Term, Type, TypeError};

use super::*;
use crate::elaborate::CoreDef;
use crate::span::Span;

mod classify_rows;
mod context_and_render;
mod corpus_tests;
mod kernel_delta;
mod proof_bearing_tests;
mod shell;
mod two_passes;

fn eq_nat(left: Term, right: Term) -> Type {
    Type::eq(Type::Nat, left, right)
}

fn mismatch(expected: Type, got: Type) -> TypeError {
    TypeError::TypeMismatch { expected, got }
}

fn probes(lenient_equal: Option<bool>, converts: Option<bool>) -> Probes {
    Probes {
        lenient_equal,
        converts,
        ..Probes::default()
    }
}

/// A `ProbeEngine` that answers every refusal with the same probes.
struct FixedProbes(Probes);

impl ProbeEngine for FixedProbes {
    fn probes_for(&self, _: &TypeError) -> Probes {
        self.0
    }
}

// ── The tempdir corpus `corpus_tests` and `proof_bearing_tests` share ──

/// 2.10.26o AC2's corpus: three entries reaching modules (one in another
/// directory), two entries sharing a module, an empty file, a broken one, a
/// companion and a hidden directory.
const CORPUS: [(&str, &str); 12] = [
    (
        "app/main.tg",
        "pub mod util;\ntheorem one(): 1 == 1 { refl }\nfn main() -> Nat { 0 }\n",
    ),
    (
        "app/util/mod.tg",
        "pub mod deep;\npub fn id(n: Nat) -> Nat { n }\n",
    ),
    (
        "app/util/deep/mod.tg",
        // A record theorem: still a `named-type` refusal after K-δ (ADR
        // 2.10.26l), whose δ example `two_is_two` the kernel now admits.
        // Self-recursive, so 3.10.26p's presentation leaves it named (D3).
        "type Point = { x: Nat, next: Link }\ntype Link = End | More(Point)\n\
         fn px(p: Point) -> Nat { p.x }\n\
         theorem px_refl(p: Point): px(p) == px(p) { refl }\n",
    ),
    ("shared/a.tg", "pub mod common;\nfn main() -> Nat { 0 }\n"),
    ("shared/b.tg", "pub mod common;\nfn main() -> Nat { 1 }\n"),
    ("shared/common.tg", "theorem same(): 2 == 2 { refl }\n"),
    (
        "shared/common_test.tg",
        "theorem in_companion(): 3 == 3 { refl }\n",
    ),
    ("far/main.tg", "pub mod sub;\nfn main() -> Nat { 0 }\n"),
    ("far/sub/mod.tg", "theorem far_away(): 4 == 4 { refl }\n"),
    ("plain/empty.tg", "fn main() -> Nat { 0 }\n"),
    (
        "bad/broken.tg",
        "fn main() -> Nat { true }\ntheorem lost(): 5 == 5 { refl }\n",
    ),
    (".hidden/secret.tg", "theorem hidden(): 6 == 6 { refl }\n"),
];

fn write(root: &std::path::Path, relative: &str, source: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    std::fs::write(path, source).expect("write");
}

fn corpus() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = std::fs::canonicalize(dir.path()).expect("canonical");
    for (relative, source) in CORPUS {
        write(&root, relative, source);
    }
    (dir, root)
}

fn register(rows: &str) -> crate::elaborate::kernel::Register {
    crate::elaborate::kernel::Register::parse(&format!("schema = 1\n{rows}"))
        .expect("a fixture register")
}

fn needs_delta_row(file: &str, theorem: &str) -> String {
    format!("[[exemption]]\nfile = \"{file}\"\ntheorem = \"{theorem}\"\ncause = \"needs-delta\"\nremoved_by = \"2.10.26l\"\n")
}
