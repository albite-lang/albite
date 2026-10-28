use super::*;

#[test]
fn test_format_bytes() {
    assert_eq!(report::format_bytes(0), "0");
    assert_eq!(report::format_bytes(999), "999");
    assert_eq!(report::format_bytes(1000), "1,000");
    assert_eq!(report::format_bytes(1_000_000), "1,000,000");
    assert_eq!(report::format_bytes(1_432_871_424), "1,432,871,424");
}

// ADR 30.9.26l AC6: the shared percentage helper the four printers call.
#[test]
fn percent_of_is_the_share_times_a_hundred() {
    let close = |got: f64, want: f64| (got - want).abs() < 1e-9;
    assert!(close(report::percent_of(1, 4), 25.0));
    assert!(close(report::percent_of(3, 4), 75.0));
    assert!(close(report::percent_of(4, 4), 100.0));
    assert!(close(report::percent_of(0, 4), 0.0));
}

#[test]
fn test_profiler_record() {
    let mut profiler = Profiler::new();
    let name = c"test_fn".as_ptr();
    profiler.current_fn = name;
    profiler.record(100, CLASS_MU);
    profiler.record(200, CLASS_MU);

    assert_eq!(profiler.total_bytes, 300);
    assert_eq!(profiler.total_count, 2);
    assert_eq!(profiler.entry_count, 1);
    assert_eq!(profiler.entries[0].bytes, 300);
    assert_eq!(profiler.entries[0].count, 2);
    assert!(profiler.active);
}

#[test]
fn test_profiler_multiple_functions() {
    let mut profiler = Profiler::new();
    let fn_a = c"fn_a".as_ptr();
    let fn_b = c"fn_b".as_ptr();

    profiler.current_fn = fn_a;
    profiler.record(100, CLASS_MU);

    profiler.current_fn = fn_b;
    profiler.record(500, CLASS_ENV);

    profiler.current_fn = fn_a;
    profiler.record(200, CLASS_STRING);

    assert_eq!(profiler.total_bytes, 800);
    assert_eq!(profiler.entry_count, 2);
    assert_eq!(profiler.entries[0].bytes, 300); // fn_a
    assert_eq!(profiler.entries[1].bytes, 500); // fn_b
}

#[test]
fn test_profiler_class_accounting() {
    let mut profiler = Profiler::new();
    let name = c"fn_classes".as_ptr();
    profiler.current_fn = name;

    profiler.record(100, CLASS_MU);
    profiler.record(200, CLASS_ENV);
    profiler.record(300, CLASS_REF);
    profiler.record(400, CLASS_STRING);
    profiler.record(500, CLASS_OTHER);

    assert_eq!(profiler.class_bytes[CLASS_MU as usize], 100);
    assert_eq!(profiler.class_bytes[CLASS_ENV as usize], 200);
    assert_eq!(profiler.class_bytes[CLASS_REF as usize], 300);
    assert_eq!(profiler.class_bytes[CLASS_STRING as usize], 400);
    assert_eq!(profiler.class_bytes[CLASS_OTHER as usize], 500);
    assert_eq!(profiler.class_count[CLASS_MU as usize], 1);

    // Per-function per-class attribution
    assert_eq!(
        profiler.entries[0].bytes_by_class[CLASS_STRING as usize],
        400
    );
    assert_eq!(profiler.entries[0].bytes, 1500);
}

#[test]
fn test_profiler_out_of_range_class_clamps_to_other() {
    let mut profiler = Profiler::new();
    profiler.record(64, 999);
    assert_eq!(profiler.class_bytes[CLASS_OTHER as usize], 64);
}

#[test]
fn test_mu_bucket_boundaries() {
    assert_eq!(mu_bucket(1), 0);
    assert_eq!(mu_bucket(16), 0);
    assert_eq!(mu_bucket(17), 1);
    assert_eq!(mu_bucket(32), 1);
    assert_eq!(mu_bucket(64), 2);
    assert_eq!(mu_bucket(128), 3);
    assert_eq!(mu_bucket(256), 4);
    assert_eq!(mu_bucket(257), 5);
    assert_eq!(mu_bucket(1 << 20), 5);
}

#[test]
fn test_mu_histogram_only_counts_mu_class() {
    let mut profiler = Profiler::new();
    profiler.record(24, CLASS_MU);
    profiler.record(24, CLASS_ENV);
    profiler.record(300, CLASS_MU);

    assert_eq!(profiler.mu_size_hist[1], 1); // 24B → ≤32 bucket
    assert_eq!(profiler.mu_size_hist[NUM_MU_BUCKETS - 1], 1); // 300B → overflow
    let total: u64 = profiler.mu_size_hist.iter().sum();
    assert_eq!(total, 2);
}

/// ADR 29.9.26h AC 2: the nullary fold class lands in its own index 4, and
/// the clamp still lands an out-of-range class on `other`, now index 5.
#[test]
fn nullary_class_records_at_index_four_and_other_stays_last() {
    assert_eq!(CLASS_MU_NULLARY, 4);
    assert_eq!(CLASS_OTHER, 5);
    assert_eq!(CLASS_OTHER as usize, NUM_CLASSES - 1);
    let mut profiler = Profiler::new();
    profiler.record(16, CLASS_MU_NULLARY);
    profiler.record(40, 6);
    assert_eq!(profiler.class_count[4], 1);
    assert_eq!(profiler.class_bytes[4], 16);
    assert_eq!(profiler.class_bytes[5], 40);
    assert_eq!(profiler.class_count[CLASS_MU as usize], 0);
    assert_eq!(CLASS_NAMES[4], "mu_alloc nullary (ADT)");
}

/// ADR 29.9.26h AC 2 / D7: both fold classes feed the `mu_alloc` histogram
/// and no other class does.
#[test]
fn both_fold_classes_feed_the_mu_histogram_and_nothing_else() {
    let mut profiler = Profiler::new();
    profiler.record(16, CLASS_MU_NULLARY);
    profiler.record(24, CLASS_MU);
    for class in [CLASS_ENV, CLASS_REF, CLASS_STRING, CLASS_OTHER, 99] {
        profiler.record(24, class);
    }
    assert_eq!(profiler.mu_size_hist[0], 1); // 16B nullary
    assert_eq!(profiler.mu_size_hist[1], 1); // 24B mu
    assert_eq!(profiler.mu_size_hist.iter().sum::<u64>(), 2);
    assert!(is_fold_class(CLASS_MU) && is_fold_class(CLASS_MU_NULLARY));
    assert!(!is_fold_class(CLASS_OTHER) && !is_fold_class(CLASS_STRING));
}

/// ADR 29.9.26h AC 2: `class_line` prints `nullary=` only when non-zero.
#[test]
fn class_line_prints_nullary_only_when_non_zero() {
    let mut profiler = Profiler::new();
    profiler.record(1000, CLASS_MU);
    assert_eq!(report::class_line(&profiler), "mu=1,000");
    profiler.record(16, CLASS_MU_NULLARY);
    profiler.record(8, CLASS_OTHER);
    assert_eq!(report::class_line(&profiler), "mu=1,000 nullary=16 other=8");
}

/// ADR 29.9.26h AC 2: `dominant_class` can name `nullary`, and `other` keeps
/// its key at the new last index.
#[test]
fn dominant_class_can_name_nullary() {
    let mut by_class = [0u64; NUM_CLASSES];
    by_class[CLASS_MU as usize] = 10;
    by_class[CLASS_MU_NULLARY as usize] = 20;
    assert_eq!(report::dominant_class(&by_class), "nullary");
    by_class[CLASS_OTHER as usize] = 30;
    assert_eq!(report::dominant_class(&by_class), "other");
    assert_eq!(report::dominant_class(&[0; NUM_CLASSES]), "-");
}

/// ADR 29.9.26h AC 3's runtime half: no short key contains another, so the
/// marker parser's `key=` substring search cannot misread one as another.
#[test]
fn no_class_short_key_contains_another() {
    for (i, a) in CLASS_SHORT.iter().enumerate() {
        for (j, b) in CLASS_SHORT.iter().enumerate() {
            if i != j {
                assert!(
                    !format!("{a}=").contains(&format!("{b}=")),
                    "{a} contains {b}"
                );
            }
        }
    }
}

#[test]
fn test_profiler_no_current_fn() {
    let mut profiler = Profiler::new();
    profiler.record(100, CLASS_MU);

    assert_eq!(profiler.total_bytes, 100);
    assert_eq!(profiler.entry_count, 0); // no attribution
                                         // class accounting still works without a current function
    assert_eq!(profiler.class_bytes[CLASS_MU as usize], 100);
}

#[test]
fn test_profiler_last_entry_fast_path() {
    let mut profiler = Profiler::new();
    let fn_a = c"fn_a".as_ptr();
    let fn_b = c"fn_b".as_ptr();

    profiler.current_fn = fn_a;
    profiler.record(10, CLASS_MU);
    profiler.current_fn = fn_b;
    profiler.record(20, CLASS_MU);
    // Back to fn_a — must find the existing entry, not create a duplicate
    profiler.current_fn = fn_a;
    profiler.record(30, CLASS_MU);

    assert_eq!(profiler.entry_count, 2);
    assert_eq!(profiler.entries[0].bytes, 40);
    assert_eq!(profiler.entries[1].bytes, 20);
}

#[test]
fn test_profiler_overflow_no_panic() {
    let mut profiler = Profiler::new();
    // Create MAX_FUNCTIONS distinct function name pointers.
    let names: Vec<*const c_char> = (0..MAX_FUNCTIONS)
        .map(|i| {
            // Use heap-allocated CStrings to guarantee distinct pointers
            let s = std::ffi::CString::new(format!("fn_{}", i)).unwrap();
            let ptr = s.as_ptr();
            std::mem::forget(s); // leak to keep pointer valid
            ptr
        })
        .collect();

    for &name in &names {
        profiler.current_fn = name;
        profiler.record(10, CLASS_MU);
    }
    assert_eq!(profiler.entry_count, MAX_FUNCTIONS);
    assert_eq!(profiler.total_bytes, (MAX_FUNCTIONS as u64) * 10);

    // One more should not panic — allocation is tracked in total but not per-fn
    let overflow_name = std::ffi::CString::new("fn_overflow").unwrap();
    profiler.current_fn = overflow_name.as_ptr();
    profiler.record(99, CLASS_MU);
    assert_eq!(profiler.entry_count, MAX_FUNCTIONS); // still at max
    assert_eq!(profiler.total_bytes, (MAX_FUNCTIONS as u64) * 10 + 99);
}

#[test]
fn test_profiler_filter() {
    let mut profiler = Profiler::new();
    let fn_a = c"fn_a".as_ptr();
    let fn_b = c"fn_b".as_ptr();

    profiler.current_fn = fn_a;
    profiler.record(100, CLASS_MU);
    profiler.current_fn = fn_b;
    profiler.record(500, CLASS_MU);

    // Setting a filter stores the pointer for report-time filtering
    profiler.filter_fn = fn_a;
    assert!(!profiler.filter_fn.is_null());
}

#[test]
fn test_external_record_inactive_is_noop() {
    // The global profiler starts inactive; external records must be dropped
    // so non-profiled builds (e.g. the L1 bootstrap linking albite_core)
    // never accumulate state. Note: uses the global PROFILER — other tests
    // in this file use local Profiler instances so there is no interference.
    alloc_profile_record_external(1234, CLASS_STRING);
    unsafe {
        let profiler = &*PROFILER.0.get();
        assert!(!profiler.active);
        assert_eq!(profiler.total_bytes, 0);
    }
}

#[test]
fn test_interim_dump_threshold_advances() {
    let mut profiler = Profiler::new();
    profiler.dump_interval_initialized = true;
    profiler.dump_interval = 1000;
    profiler.next_dump_bytes = 1000;
    profiler.current_fn = c"fn_dump".as_ptr();

    // Crossing multiple intervals in one jump must advance past total_bytes
    profiler.record(3500, CLASS_MU);
    assert!(profiler.next_dump_bytes > profiler.total_bytes);
    assert_eq!(profiler.next_dump_bytes, 4000);
}

#[test]
fn test_interim_dump_disabled() {
    let mut profiler = Profiler::new();
    profiler.dump_interval_initialized = true;
    profiler.dump_interval = 0;
    profiler.record(10_000, CLASS_MU);
    assert_eq!(profiler.next_dump_bytes, 0);
}

/// ADR 14.9.26b AC 8: the bump tail's exact text — the keys the
/// `selfcompiled-profile` parser is promised, the mode rendered as `off`/`on`,
/// and thousands separators on the byte counts.
#[test]
fn bump_fields_render_mode_and_counts() {
    use crate::alloc::{ArenaStatsOut, MODE_BUMP, MODE_OFF};
    let on = ArenaStatsOut {
        mode: MODE_BUMP,
        chunks: 3,
        reserved: 5_097_152,
        used: 3_025_188,
        high_water: 3_025_188,
    };
    assert_eq!(
        report::bump_fields(&on),
        "bump=on chunks=3 reserved=5,097,152 used=3,025,188 hw=3,025,188"
    );
    let off = ArenaStatsOut {
        mode: MODE_OFF,
        ..ArenaStatsOut::default()
    };
    assert_eq!(
        report::bump_fields(&off),
        "bump=off chunks=0 reserved=0 used=0 hw=0"
    );
    assert_eq!(
        report::bump_summary(&off),
        "arena: bump=off chunks=0 reserved=0 used=0 hw=0 (heap allocations through the runtime symbol only; stack folds excluded)"
    );
}

/// ADR 20.10.26e: the interim-dump interval is read through the settings seam
/// under either spelling, in MB, and defaults when neither is set. One test,
/// so no other test races it on the same two variables.
#[test]
fn dump_interval_reads_either_spelling_and_defaults_when_unset() {
    let current = "ALBITE_ALLOC_PROFILE_INTERVAL_MB";
    let legacy = crate::settings::legacy_name("ALLOC_PROFILE_INTERVAL_MB");
    // SAFETY: the environment is process-global; no other test reads or
    // writes these two variables, and both are removed before returning.
    unsafe { std::env::set_var(current, "3") };
    assert_eq!(dump_interval_from_env(), 3 * 1024 * 1024);
    unsafe { std::env::remove_var(current) };
    unsafe { std::env::set_var(&legacy, "5") };
    assert_eq!(dump_interval_from_env(), 5 * 1024 * 1024);
    unsafe { std::env::remove_var(&legacy) };
    assert_eq!(dump_interval_from_env(), DEFAULT_DUMP_INTERVAL_BYTES);
}
