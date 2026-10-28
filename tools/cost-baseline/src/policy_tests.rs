//! 15.10.26f AC3: the trigger, the staleness test and the bound rule.

use crate::policy::{applies, bound_change, stale_fields, BoundChange, Field, TRIGGER_PREFIXES};
use crate::record::RECORD_PATH;
use crate::{Bound, Environment};

type Widen = fn(&mut Bound);

fn bound() -> Bound {
    Bound {
        wall_pct: 10.0,
        rss_pct: 10.0,
        noise_pct: 8.0,
        drift_ceiling_pct: 20.0,
        stored_wall_margin_pct: 2.0,
        runs: 5,
        adr: "15.10.26f".into(),
    }
}

fn environment() -> Environment {
    Environment {
        kernel: "6.10.14-linuxkit".into(),
        os: "Ubuntu 24.04.1 LTS".into(),
        cpu_model: "0xd4f".into(),
        llc: "Ubuntu LLVM version 18.1.3".into(),
        rustc: "rustc 1.90.0".into(),
        nproc: 12,
        mem_total_kb: 16_000_000,
    }
}

/// 15.10.26f AC3: each of D9's six prefixes triggers, and so does the record.
#[test]
fn applies_to_each_trigger_prefix_and_the_record() {
    for prefix in TRIGGER_PREFIXES {
        let path = format!("{prefix}x/mod.rs");
        assert!(applies(&[path.as_str()]), "{path}");
    }
    assert!(applies(&[RECORD_PATH]));
    assert!(
        applies(&["src/compiler/driver/ffi/test.tg"]),
        "an imported test MODULE triggers"
    );
    assert!(
        applies(&["docs/x.md", "src/compiler/main.tg"]),
        "one triggering path is enough"
    );
}

/// 20.10.26g AC5: the renamed core and code-generator directories trigger the
/// self-host's check-cost gate, as the directories they were moved from did.
#[test]
fn applies_to_the_renamed_crate_directories() {
    assert!(applies(&["albite_core/src/types/mod.rs"]));
    assert!(applies(&["albite_codegen/src/codegen/mod.rs"]));
    assert!(
        !applies(&["albite_runtime/src/lib.rs"]),
        "the runtime never triggered"
    );
}

/// 15.10.26f AC3: the elaborator, docs, notes and (P0.6) the suites do not.
#[test]
fn applies_is_false_outside_the_prefixes_and_for_suite_files() {
    for path in [
        "bootstrap/src/elaborate/mod.rs",
        "docs/repo-memory/index.md",
        "notes/x.md",
        "src/compiler/test_codegen.tg",
        "src/compiler/mustfail_ast_compare.tg",
        "tools/albite-dev/src/commands/check_cost/mod.rs",
        "tools/tungsten1-check-cost.toml.bak",
    ] {
        assert!(!applies(&[path]), "{path}");
    }
    assert!(!applies(&[]));
    assert!(
        applies(&["src/compiler/test_codegen.rs"]),
        "only .tg suites are excluded"
    );
    assert!(
        applies(&["src/compiler/lexer/test_helpers/mod.tg"]),
        "a directory named test_ is not a suite"
    );
}

/// 15.10.26f AC3: `stale_fields` names exactly the fields that differ.
#[test]
fn stale_fields_names_exactly_the_differing_fields() {
    let recorded = environment();
    assert!(stale_fields(&recorded, &recorded).is_empty());

    let mut current = environment();
    current.cpu_model = "0xd4e".into();
    current.kernel = "6.11.0".into();
    assert_eq!(
        stale_fields(&recorded, &current),
        vec![Field::Kernel, Field::CpuModel]
    );

    let mut all = environment();
    all.os = "Debian".into();
    all.llc = "LLVM 19".into();
    all.rustc = "rustc 1.91.0".into();
    all.nproc = 8;
    all.mem_total_kb = 8_000_000;
    let names: Vec<&str> = stale_fields(&recorded, &all)
        .iter()
        .map(|f| f.name())
        .collect();
    assert_eq!(names, ["os", "llc", "rustc", "nproc", "mem_total_kb"]);
    assert_eq!(Field::Kernel.name(), "kernel");
    assert_eq!(Field::CpuModel.name(), "cpu_model");
}

/// 15.10.26f AC3: with `adr` unchanged, each loosening is refused by name.
#[test]
fn bound_change_refuses_each_widening_with_the_same_adr() {
    let base = bound();
    let cases: [(&str, Widen); 6] = [
        ("wall_pct", |b| b.wall_pct = 12.0),
        ("rss_pct", |b| b.rss_pct = 10.5),
        ("noise_pct", |b| b.noise_pct = 9.0),
        ("drift_ceiling_pct", |b| b.drift_ceiling_pct = 25.0),
        ("stored_wall_margin_pct", |b| b.stored_wall_margin_pct = 1.0),
        ("runs", |b| b.runs = 3),
    ];
    for (name, widen) in cases {
        let mut tree = bound();
        widen(&mut tree);
        assert_eq!(
            bound_change(&base, &tree),
            BoundChange::Widened(vec![name]),
            "{name}"
        );
    }
}

/// 15.10.26f AC3: a tightening passes, and so does a widening naming a new ADR.
#[test]
fn bound_change_accepts_a_tightening_and_a_new_adr() {
    let base = bound();
    assert_eq!(bound_change(&base, &bound()), BoundChange::Unchanged);

    let mut tighter = bound();
    tighter.wall_pct = 8.0;
    tighter.rss_pct = 5.0;
    tighter.noise_pct = 6.0;
    tighter.drift_ceiling_pct = 15.0;
    tighter.stored_wall_margin_pct = 10.0;
    tighter.runs = 7;
    assert_eq!(bound_change(&base, &tighter), BoundChange::Accepted);

    let mut widened_by_adr = bound();
    widened_by_adr.wall_pct = 15.0;
    widened_by_adr.adr = "1.11.26a".into();
    assert_eq!(bound_change(&base, &widened_by_adr), BoundChange::Accepted);

    let mut equal_values_new_adr = bound();
    equal_values_new_adr.adr = "1.11.26a".into();
    assert_eq!(
        bound_change(&base, &equal_values_new_adr),
        BoundChange::Accepted
    );
}
