//! 15.10.26f AC2: the verdict, against 13.10.26k's own numbers.
//!
//! §6.1's wall rows are used verbatim. §6 reports HEAD's wall and RSS as one
//! number each, so that arm is the number three times, and every arm whose RSS
//! §6/§6.1 report only as a range is the range's two endpoints plus their
//! midpoint as a SYNTHETIC third sample — likewise §6's padded wall range.

use crate::policy::Field;
use crate::record::RecordState;
use crate::verdict::{
    classify_stored, compare, delta_pct, median, plan, resolve_ab, spread_pct, Comparison, Next,
    PlanInputs, StoredClass,
};
use crate::{Bound, Samples, Verdict};

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

fn arm(wall_s: &[f64], max_rss_kb: &[u64]) -> Samples {
    Samples {
        wall_s: wall_s.to_vec(),
        max_rss_kb: max_rss_kb.to_vec(),
        exit_codes: vec![0; wall_s.len()],
    }
}

/// 13.10.26k §6: HEAD's match files.
fn section6_head() -> Samples {
    arm(&[14.30, 14.30, 14.30], &[1_665_472, 1_665_472, 1_665_472])
}

/// 13.10.26k §6: padded (D1+D2), 16.21–16.35 s and 1,956,736–1,958,124 KB.
fn section6_padded() -> Samples {
    arm(&[16.21, 16.28, 16.35], &[1_956_736, 1_957_430, 1_958_124])
}

/// 13.10.26k §6.1: HEAD / HEAD source.
fn section61_head() -> Samples {
    arm(&[15.29, 14.78, 14.65], &[1_676_620, 1_676_836, 1_677_052])
}

/// 13.10.26k §6.1: amended / amended source.
fn section61_amended() -> Samples {
    arm(&[15.85, 15.59, 15.41], &[1_777_604, 1_778_752, 1_779_900])
}

fn comparison(verdict: Verdict, wall_pct: f64, rss_pct: f64) -> Comparison {
    Comparison {
        verdict,
        wall_pct,
        rss_pct,
        wall_spread_pct: 0.9,
        rss_spread_pct: 0.1,
    }
}

#[test]
fn median_and_spread_follow_d1() {
    assert!((median(&[3.0, 1.0, 2.0]) - 2.0).abs() < 1e-12);
    assert!((median(&[4.0, 1.0, 2.0, 3.0]) - 2.5).abs() < 1e-12);
    assert!(median(&[]).abs() < 1e-12);
    // (x[3] − x[1]) / x[2] over the sorted five: (12 − 10) / 11.
    let five = [13.0, 10.0, 11.0, 12.0, 9.0];
    assert!((spread_pct(&five) - 2.0 / 11.0 * 100.0).abs() < 1e-9);
    assert!(spread_pct(&[]).abs() < 1e-12);
    assert!(spread_pct(&[0.0, 0.0, 0.0]).abs() < 1e-12);
    // Under five samples the inset is 0, so the spread is the full range.
    assert!((spread_pct(&[1.0, 2.0, 3.0, 4.0]) - 120.0).abs() < 1e-9);
    assert!((delta_pct(&[10.0], &[11.0]) - 10.0).abs() < 1e-9);
    assert!((delta_pct(&[10.0], &[9.0]) + 10.0).abs() < 1e-9);
    assert!(delta_pct(&[0.0], &[5.0]).abs() < 1e-12);
}

/// 15.10.26f AC2: §6's padded arm against its HEAD arm regresses on both.
#[test]
fn section6_padded_arm_is_regressed_on_both_metrics() {
    let c = compare(&section6_head(), &section6_padded(), &bound());
    assert_eq!(c.verdict, Verdict::Regressed);
    assert!(c.wall_pct > 10.0, "wall {}", c.wall_pct);
    assert!(c.rss_pct > 10.0, "rss {}", c.rss_pct);
    assert!(
        (c.wall_pct - 13.85).abs() < 0.05,
        "+14% wall: {}",
        c.wall_pct
    );
    assert!(
        (c.rss_pct - 17.53).abs() < 0.05,
        "+17.5% rss: {}",
        c.rss_pct
    );
}

/// 15.10.26f AC2: §6.1's amended arm passes, at +5.5% wall.
#[test]
fn section61_amended_arm_passes_at_five_and_a_half_percent() {
    let c = compare(&section61_head(), &section61_amended(), &bound());
    assert_eq!(c.verdict, Verdict::Pass);
    assert!((c.wall_pct - 5.48).abs() < 0.05, "wall {}", c.wall_pct);
    assert!((c.rss_pct - 6.1).abs() < 0.05, "rss {}", c.rss_pct);
    // §6.1's HEAD arm: a 4.3% spread, under the noise ceiling.
    assert!(
        (c.wall_spread_pct - 4.33).abs() < 0.05,
        "spread {}",
        c.wall_spread_pct
    );
}

/// 15.10.26f AC2: a one-metric rise past the bound is enough.
#[test]
fn either_metric_past_the_bound_regresses() {
    let base = arm(&[10.0; 5], &[1_000; 5]);
    let rss_only = arm(&[10.0; 5], &[1_101; 5]);
    assert_eq!(
        compare(&base, &rss_only, &bound()).verdict,
        Verdict::Regressed
    );
    let wall_only = arm(&[11.01; 5], &[1_000; 5]);
    assert_eq!(
        compare(&base, &wall_only, &bound()).verdict,
        Verdict::Regressed
    );
    let at_bound = arm(&[11.0; 5], &[1_100; 5]);
    assert_eq!(
        compare(&base, &at_bound, &bound()).verdict,
        Verdict::Pass,
        "10.0% is within"
    );
}

/// 15.10.26f AC2: a 9% interquartile wall spread is noisy in either arm, even
/// with its median within the bound; two far outliers around a tight middle
/// three are not.
#[test]
fn a_nine_percent_spread_is_noisy_in_either_arm() {
    let quiet = arm(&[10.0, 10.0, 10.0, 10.0, 10.0], &[1_000; 5]);
    // sorted 9.5, 9.6, 10.0, 10.5, 10.6: (10.5 − 9.6) / 10.0 = 9%.
    let nine = arm(&[10.0, 9.6, 10.5, 9.5, 10.6], &[1_000; 5]);
    assert!((spread_pct(&nine.wall_s) - 9.0).abs() < 1e-9);
    assert_eq!(compare(&quiet, &nine, &bound()).verdict, Verdict::Noisy);
    assert_eq!(compare(&nine, &quiet, &bound()).verdict, Verdict::Noisy);

    let outliers = arm(&[6.0, 10.0, 10.1, 9.9, 15.0], &[1_000; 5]);
    let c = compare(&quiet, &outliers, &bound());
    assert_eq!(c.verdict, Verdict::Pass, "spread {}", c.wall_spread_pct);

    let noisy_rss = arm(&[10.0; 5], &[900, 950, 1_000, 1_050, 1_100]);
    assert_eq!(
        compare(&quiet, &noisy_rss, &bound()).verdict,
        Verdict::Noisy
    );
}

/// 15.10.26f AC2: a non-zero exit in any sample, in either arm, is check-failed.
#[test]
fn a_non_zero_exit_is_check_failed() {
    let good = arm(&[10.0; 5], &[1_000; 5]);
    let mut bad = good.clone();
    bad.exit_codes[3] = 1;
    assert_eq!(compare(&good, &bad, &bound()).verdict, Verdict::CheckFailed);
    assert_eq!(compare(&bad, &good, &bound()).verdict, Verdict::CheckFailed);
    assert_eq!(
        compare(&good, &Samples::default(), &bound()).verdict,
        Verdict::CheckFailed
    );
    assert_eq!(
        compare(&Samples::default(), &good, &bound()).verdict,
        Verdict::CheckFailed
    );
}

/// 15.10.26f AC2: a stored wall delta between `wall_pct − margin` and
/// `wall_pct` is inconclusive; past the bound and past the ceiling are not.
#[test]
fn stored_deltas_classify_against_margin_bound_and_ceiling() {
    let b = bound();
    let pass = |w, r| comparison(Verdict::Pass, w, r);
    assert_eq!(classify_stored(&pass(7.9, 9.0), &b), StoredClass::Pass);
    assert_eq!(
        classify_stored(&pass(8.0, 0.0), &b),
        StoredClass::Inconclusive
    );
    assert_eq!(
        classify_stored(&pass(9.9, 0.0), &b),
        StoredClass::Inconclusive
    );
    let regressed = |w, r| comparison(Verdict::Regressed, w, r);
    assert_eq!(
        classify_stored(&regressed(12.0, 1.0), &b),
        StoredClass::PastBound
    );
    assert_eq!(
        classify_stored(&regressed(20.0, 20.0), &b),
        StoredClass::PastBound
    );
    assert_eq!(
        classify_stored(&regressed(20.1, 1.0), &b),
        StoredClass::PastDriftCeiling
    );
    assert_eq!(
        classify_stored(&regressed(1.0, 20.1), &b),
        StoredClass::PastDriftCeiling
    );
    let noisy = comparison(Verdict::Noisy, 30.0, 0.0);
    assert_eq!(
        classify_stored(&noisy, &b),
        StoredClass::Stops(Verdict::Noisy)
    );
    let failed = comparison(Verdict::CheckFailed, 0.0, 0.0);
    assert_eq!(
        classify_stored(&failed, &b),
        StoredClass::Stops(Verdict::CheckFailed)
    );
}

fn present() -> RecordState {
    let text = "\
[bound]
wall_pct = 10.0
rss_pct = 10.0
noise_pct = 8.0
drift_ceiling_pct = 20.0
stored_wall_margin_pct = 2.0
runs = 5
adr = \"15.10.26f\"

[measured]
commit = \"abc\"
blessed_on = \"2026-10-16\"
wall_s = [25.0, 25.1, 25.2, 25.0, 25.1]
max_rss_kb = [1985624, 1986512, 1985932, 1985700, 1986000]

[environment]
kernel = \"k\"
os = \"o\"
cpu_model = \"c\"
llc = \"l\"
rustc = \"r\"
nproc = 12
mem_total_kb = 16000000
";
    RecordState::from_text(Some(text))
}

fn inputs<'a>(
    record: &'a RecordState,
    stale: &'a [Field],
    stored: Option<StoredClass>,
) -> PlanInputs<'a> {
    PlanInputs {
        applies: true,
        record,
        stale,
        stored,
        no_ab: false,
        force_ab: false,
    }
}

/// 15.10.26f AC2: `plan` takes each arrow of §2.1.
#[test]
fn plan_takes_each_arrow() {
    let record = present();
    assert!(
        matches!(record, RecordState::Present(_)),
        "the committed record parses"
    );
    let mut not_applicable = inputs(&record, &[], None);
    not_applicable.applies = false;
    assert_eq!(plan(&not_applicable), Next::Decided(Verdict::NotApplicable));

    assert_eq!(plan(&inputs(&record, &[], None)), Next::MeasureStored);
    let stored_pass = inputs(&record, &[], Some(StoredClass::Pass));
    assert_eq!(plan(&stored_pass), Next::Decided(Verdict::Pass));
    for class in [
        StoredClass::Inconclusive,
        StoredClass::PastBound,
        StoredClass::PastDriftCeiling,
    ] {
        assert_eq!(
            plan(&inputs(&record, &[], Some(class))),
            Next::RunAb,
            "{class:?}"
        );
    }
    let noisy = inputs(&record, &[], Some(StoredClass::Stops(Verdict::Noisy)));
    assert_eq!(plan(&noisy), Next::Decided(Verdict::Noisy));

    assert_eq!(
        plan(&inputs(&record, &[Field::Kernel], None)),
        Next::RunAb,
        "stale → A/B"
    );
    let absent = RecordState::Absent;
    assert_eq!(
        plan(&inputs(&absent, &[], None)),
        Next::RunAb,
        "absent → A/B"
    );
    let malformed = RecordState::Malformed("bad".into());
    assert_eq!(
        plan(&inputs(&malformed, &[], None)),
        Next::RunAb,
        "malformed → A/B"
    );
}

/// 15.10.26f AC2: `--force-ab` runs the A/A after a stored pass, `--no-ab`
/// makes the file comparison the verdict.
#[test]
fn plan_honours_force_ab_and_no_ab() {
    let record = present();
    let mut forced = inputs(&record, &[], Some(StoredClass::Pass));
    forced.force_ab = true;
    assert_eq!(plan(&forced), Next::RunAb);

    let mut no_ab = inputs(&record, &[], Some(StoredClass::PastBound));
    no_ab.no_ab = true;
    assert_eq!(plan(&no_ab), Next::Decided(Verdict::Regressed));
    no_ab.stored = Some(StoredClass::PastDriftCeiling);
    assert_eq!(plan(&no_ab), Next::Decided(Verdict::Regressed));
    no_ab.stored = Some(StoredClass::Inconclusive);
    assert_eq!(plan(&no_ab), Next::Decided(Verdict::Pass));
    no_ab.stored = Some(StoredClass::Pass);
    assert_eq!(plan(&no_ab), Next::Decided(Verdict::Pass));
}

/// 15.10.26f AC2: the A/B decides — fail regresses; a pass over a stored
/// failure is drift, past the ceiling drift-exceeded.
#[test]
fn resolve_ab_attributes_the_cost() {
    let clean = comparison(Verdict::Pass, 1.1, 0.2);
    let failed = comparison(Verdict::Regressed, 12.0, 0.2);
    assert_eq!(
        resolve_ab(Some(StoredClass::PastBound), &failed),
        Verdict::Regressed
    );
    assert_eq!(resolve_ab(None, &failed), Verdict::Regressed);
    assert_eq!(
        resolve_ab(Some(StoredClass::PastBound), &clean),
        Verdict::Drift
    );
    assert_eq!(
        resolve_ab(Some(StoredClass::PastDriftCeiling), &clean),
        Verdict::DriftExceeded
    );
    assert_eq!(
        resolve_ab(Some(StoredClass::Inconclusive), &clean),
        Verdict::Pass
    );
    assert_eq!(resolve_ab(None, &clean), Verdict::Pass);
    let noisy = comparison(Verdict::Noisy, 0.0, 0.0);
    assert_eq!(
        resolve_ab(Some(StoredClass::PastBound), &noisy),
        Verdict::Noisy
    );
}

/// 15.10.26f AC2: "past" a bound is strict — a delta or a spread exactly AT
/// its bound passes, on each of the four arms of `compare`.
#[test]
fn a_delta_or_spread_exactly_at_its_bound_passes() {
    let at = Bound {
        wall_pct: 25.0,
        rss_pct: 25.0,
        noise_pct: 100.0,
        ..bound()
    };
    let wall_up = compare(&arm(&[4.0; 3], &[8; 3]), &arm(&[5.0; 3], &[8; 3]), &at);
    let rss_up = compare(&arm(&[4.0; 3], &[4; 3]), &arm(&[4.0; 3], &[5; 3]), &at);
    let spread = arm(&[1.0, 1.0, 2.0, 3.0, 3.0], &[1, 1, 2, 3, 3]);
    let at_noise = compare(&spread, &spread, &at);
    for c in [wall_up, rss_up, at_noise] {
        assert_eq!(c.verdict, Verdict::Pass, "{c:?}");
    }
}
