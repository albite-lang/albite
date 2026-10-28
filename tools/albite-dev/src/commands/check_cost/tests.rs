//! 15.10.26f AC4: the measurement seam, tested without a self-compile.

use super::bless::{bless_refusal, samples_refusal};
use super::environment::{
    parse_cpuinfo_part, parse_llc_version, parse_lscpu_model, parse_mem_total_kb, parse_nproc,
    parse_os_release, parse_trimmed,
};
use super::measure::{raise_stack_limit, run_order, sample, Step};
use super::{seed_bound, GateArgs};
use cost_baseline::Samples;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 15.10.26f AC4: `run_order` interleaves as D5 says — warm-ups first, then
/// alternating, five each.
#[test]
fn run_order_warms_each_arm_then_alternates() {
    let order = run_order(5, 2);
    assert_eq!(order.len(), 12);
    assert_eq!(
        order[0],
        Step {
            arm: 0,
            warm_up: true
        }
    );
    assert_eq!(
        order[1],
        Step {
            arm: 1,
            warm_up: true
        }
    );
    for (i, step) in order[2..].iter().enumerate() {
        assert_eq!(
            *step,
            Step {
                arm: i % 2,
                warm_up: false
            },
            "step {i}"
        );
    }
    let stored = run_order(5, 1);
    assert_eq!(stored.len(), 6);
    assert!(stored[0].warm_up && stored[1..].iter().all(|s| !s.warm_up && s.arm == 0));
}

/// 15.10.26f AC4: each environment parser reads its field from captured text.
#[test]
fn environment_parsers_read_captured_samples() {
    assert_eq!(parse_trimmed("6.10.14-linuxkit\n"), "6.10.14-linuxkit");
    assert_eq!(
        parse_trimmed("rustc 1.90.0 (1159e78c4 2025-09-14)\n"),
        "rustc 1.90.0 (1159e78c4 2025-09-14)"
    );
    let os = "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nID=ubuntu\n";
    assert_eq!(parse_os_release(os), "Ubuntu 24.04.1 LTS");
    assert_eq!(parse_os_release("ID=x\n"), "");
    let lscpu = "Architecture:  aarch64\n  Model name:   Neoverse-N1\n";
    assert_eq!(parse_lscpu_model(lscpu), "Neoverse-N1");
    assert_eq!(parse_lscpu_model("Architecture: aarch64\n"), "");
    // Docker Desktop's arm64 VM: a placeholder, so the cpuinfo fallback runs.
    assert_eq!(parse_lscpu_model("Vendor ID: Apple\n  Model name: -\n"), "");
    let cpuinfo = "processor\t: 0\nCPU implementer\t: 0x61\nCPU part\t: 0x000\n\nprocessor\t: 1\nCPU part\t: 0x001\n";
    assert_eq!(parse_cpuinfo_part(cpuinfo), "0x000");
    assert_eq!(
        parse_llc_version("\nUbuntu LLVM version 18.1.3\n  Optimized build.\n"),
        "Ubuntu LLVM version 18.1.3"
    );
    assert_eq!(
        parse_mem_total_kb("MemTotal:       16374808 kB\nMemFree: 1 kB\n"),
        16_374_808
    );
    assert_eq!(parse_mem_total_kb("MemFree: 1 kB\n"), 0);
    assert_eq!(parse_nproc("12\n"), 12);
    assert_eq!(parse_nproc("x"), 0);
}

/// 15.10.26f AC4: one sample on a `sh` stand-in reports its exit and a
/// non-zero RSS (the `run_with_rusage_reports_status_and_rss` pattern).
#[test]
fn one_sample_reports_exit_and_rss() {
    let ok = sample(Path::new("/"), Path::new("sh"), &["-c", "exit 0"]).unwrap();
    assert_eq!(ok.exit_code, 0);
    assert!(ok.max_rss_kb > 0);
    assert!(ok.wall_s >= 0.0);
    let failed = sample(Path::new("/"), Path::new("sh"), &["-c", "exit 3"]).unwrap();
    assert_eq!(failed.exit_code, 3);
    assert!(sample(Path::new("/"), Path::new("/nonexistent-binary"), &[]).is_err());
}

/// 15.10.26f AC4: after `raise_stack_limit`, a child sees an unlimited stack.
/// A host whose hard limit is finite (macOS) cannot reach that, so there the
/// refusal is the arm asserted — `make test` runs this host-side too.
#[test]
fn raise_stack_limit_reaches_a_child() {
    let mut hard = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit writes only the struct passed.
    assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_STACK, &mut hard) }, 0);
    if hard.rlim_max != libc::RLIM_INFINITY {
        let refusal = raise_stack_limit().unwrap_err();
        assert!(refusal.contains("the hard stack limit is"), "{refusal}");
        return;
    }
    raise_stack_limit().unwrap();
    let out = Command::new("sh")
        .args(["-c", "ulimit -s"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "unlimited");
}

/// The candidate arm reads a volume copy of its `src/`: tracked and untracked
/// files, never an ignored cache, and nothing left over from a previous copy.
#[test]
fn snapshot_src_copies_the_tree_and_drops_ignored_and_stale_files() {
    let dir = tempfile::tempdir().unwrap();
    let (tree, dest) = (dir.path().join("tree"), dir.path().join("snap"));
    std::fs::create_dir_all(tree.join("src/compiler/.tungsten")).unwrap();
    let sh = |cmd: &str| {
        let ok = Command::new("sh")
            .args(["-c", cmd])
            .current_dir(&tree)
            .status()
            .unwrap();
        assert!(ok.success(), "{cmd}");
    };
    sh("git init -q && git config user.email t@t && git config user.name t");
    sh("echo .tungsten/ > .gitignore && echo a > src/compiler/main.tg && echo c > src/compiler/.tungsten/x");
    sh("echo gone > src/old.tg && git add -A && git commit -qm i && rm src/old.tg && echo n > src/new.tg");
    std::fs::create_dir_all(dest.join("src")).unwrap();
    std::fs::write(dest.join("src/stale.tg"), "s").unwrap();

    super::measure::snapshot_src(&tree, &dest).unwrap();
    assert_eq!(
        std::fs::read_to_string(dest.join("src/compiler/main.tg")).unwrap(),
        "a\n"
    );
    assert!(dest.join("src/new.tg").exists(), "untracked");
    assert!(!dest.join("src/old.tg").exists(), "deleted in the tree");
    assert!(
        !dest.join("src/stale.tg").exists(),
        "left from a previous copy"
    );
    assert!(
        !dest.join("src/compiler/.tungsten/x").exists(),
        "ignored cache"
    );
}

#[test]
fn bless_refuses_a_dirty_trigger_path_and_a_batch_rebless() {
    let clean = vec![
        "tools/tungsten1-check-cost.toml".to_string(),
        "notes/x.md".to_string(),
    ];
    assert_eq!(
        bless_refusal(&clean, false, true),
        None,
        "the record itself may differ"
    );
    assert_eq!(bless_refusal(&clean, true, false), None, "a batch may seed");
    let refusal = bless_refusal(&clean, true, true).unwrap();
    assert!(refusal.contains("human commit"), "{refusal}");
    let dirty = vec!["src/compiler/main.tg".to_string()];
    let refusal = bless_refusal(&dirty, false, false).unwrap();
    assert!(
        refusal.contains("1 path(s)") && refusal.contains("src/compiler/main.tg"),
        "{refusal}"
    );
}

#[test]
fn bless_refuses_failing_and_noisy_samples() {
    let bound = seed_bound();
    let quiet = Samples {
        wall_s: vec![25.0; 5],
        max_rss_kb: vec![1_000; 5],
        exit_codes: vec![0; 5],
    };
    assert_eq!(samples_refusal(&quiet, &bound), None);
    let mut failed = quiet.clone();
    failed.exit_codes[2] = 1;
    assert!(samples_refusal(&failed, &bound)
        .unwrap()
        .contains("non-zero"));
    assert!(samples_refusal(&Samples::default(), &bound).is_some());
    let noisy = Samples {
        wall_s: vec![20.0, 22.0, 25.0, 28.0, 30.0],
        ..quiet
    };
    assert!(samples_refusal(&noisy, &bound).unwrap().contains("noisy"));
}

#[test]
fn gate_args_carry_every_flag_but_phase_and_rebuild() {
    let args = GateArgs {
        phase: Some("measure".into()),
        base: Some("abc".into()),
        baseline_file: Some(PathBuf::from("/tmp/r.toml")),
        no_ab: true,
        force_ab: true,
        rebuild: true,
        tree: Some(PathBuf::from("/tmp/t")),
    };
    assert_eq!(
        args.carried(),
        "--base abc --baseline-file /tmp/r.toml --tree /tmp/t --no-ab --force-ab"
    );
    assert_eq!(GateArgs::default().carried(), "");
}

/// 16.10.26p D2: the base arm runs whichever of `out/albite1` and a
/// pre-16.10.26p base's root `albite1` was built last.
#[test]
fn base_binary_prefers_the_newer_of_out_and_the_legacy_root() {
    use super::ab::newer_of;
    let (cur, old) = (PathBuf::from("w/out/albite1"), PathBuf::from("w/albite1"));
    let pick = |c: Option<u32>, l: Option<u32>| newer_of(cur.clone(), c, old.clone(), l);
    assert_eq!(pick(None, None), cur);
    assert_eq!(pick(Some(1), None), cur);
    assert_eq!(pick(None, Some(1)), old);
    assert_eq!(pick(Some(2), Some(1)), cur);
    assert_eq!(pick(Some(1), Some(2)), old);
    assert_eq!(pick(Some(1), Some(1)), cur);
}
