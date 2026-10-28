//! 15.10.26f AC2: the one machine-readable line every run prints (D4).

use crate::policy::Field;
use crate::verdict::{Basis, Comparison, VerdictLine};
use crate::Verdict;

fn comparison(verdict: Verdict, wall_pct: f64, rss_pct: f64) -> Comparison {
    Comparison {
        verdict,
        wall_pct,
        rss_pct,
        wall_spread_pct: 0.9,
        rss_spread_pct: 0.1,
    }
}

fn line(verdict: Verdict, basis: Option<Basis>) -> VerdictLine {
    VerdictLine {
        verdict,
        basis,
        decided_by: Some(comparison(verdict, 1.1, 0.2)),
        stored: None,
        record: None,
        stale: Vec::new(),
        base: "abc".into(),
        tree: "f00d".into(),
    }
}

/// 15.10.26f AC2: the verdict line renders exactly, with each field.
#[test]
fn the_verdict_line_renders_exactly() {
    let mut drift = line(Verdict::Drift, Some(Basis::Ab));
    drift.stored = Some(comparison(Verdict::Regressed, 12.4, 11.0));
    assert_eq!(
        drift.render(),
        "check-cost: verdict=drift basis=ab wall=+1.1% rss=+0.2% stored=+12.4%/+11.0% \
         spread=0.9%/0.1% base=abc tree=f00d rebless-owed"
    );
    assert_eq!(
        line(Verdict::Pass, Some(Basis::Stored)).render(),
        "check-cost: verdict=pass basis=stored wall=+1.1% rss=+0.2% spread=0.9%/0.1% base=abc tree=f00d"
    );
    let mut file = line(Verdict::Regressed, Some(Basis::File));
    file.stored = Some(comparison(Verdict::Regressed, 25.0, 25.0));
    assert_eq!(
        file.render(),
        "check-cost: verdict=regressed basis=file wall=+1.1% rss=+0.2% spread=0.9%/0.1% base=abc tree=f00d",
        "stored= prints only beside an A/B"
    );
    let mut na = line(Verdict::NotApplicable, None);
    na.decided_by = None;
    assert_eq!(
        na.render(),
        "check-cost: verdict=not-applicable base=abc tree=f00d"
    );
}

/// 15.10.26f AC2: `rebless-owed` for drift, drift-exceeded, an absent record
/// and a stale pass — and not for a fresh pass or a fresh regression.
#[test]
fn rebless_owed_marks_drift_absent_and_stale_pass() {
    assert!(line(Verdict::Drift, Some(Basis::Ab)).rebless_owed());
    assert!(line(Verdict::DriftExceeded, Some(Basis::Ab)).rebless_owed());
    assert!(!line(Verdict::Pass, Some(Basis::Stored)).rebless_owed());
    assert!(!line(Verdict::Regressed, Some(Basis::Ab)).rebless_owed());

    let mut absent = line(Verdict::Pass, Some(Basis::Ab));
    absent.record = Some("absent");
    assert!(absent.rebless_owed());
    assert!(
        absent.render().contains(" record=absent "),
        "{}",
        absent.render()
    );
    let mut absent_regressed = line(Verdict::Regressed, Some(Basis::Ab));
    absent_regressed.record = Some("malformed");
    assert!(absent_regressed.rebless_owed());

    let mut stale = line(Verdict::Pass, Some(Basis::Ab));
    stale.stale = vec![Field::Kernel, Field::Llc];
    assert!(stale.rebless_owed());
    assert!(
        stale.render().contains(" stale=kernel,llc "),
        "{}",
        stale.render()
    );
    assert!(stale.render().ends_with(" rebless-owed"));
    let mut stale_regressed = line(Verdict::Regressed, Some(Basis::Ab));
    stale_regressed.stale = vec![Field::Kernel];
    assert!(
        !stale_regressed.rebless_owed(),
        "a regression is not a re-bless"
    );
}

#[test]
fn verdict_names_and_exit_codes_follow_d4() {
    let table = [
        (Verdict::Pass, "pass", 0),
        (Verdict::Drift, "drift", 0),
        (Verdict::DriftExceeded, "drift-exceeded", 1),
        (Verdict::Regressed, "regressed", 1),
        (Verdict::Noisy, "noisy", 1),
        (Verdict::CheckFailed, "check-failed", 1),
        (Verdict::Deadline, "deadline", 1),
        (Verdict::NotApplicable, "not-applicable", 0),
    ];
    for (verdict, name, exit) in table {
        assert_eq!(verdict.name(), name);
        assert_eq!(verdict.exit_code(), exit, "{name}");
    }
    assert_eq!(Basis::File.name(), "file");
    assert_eq!(Basis::Ab.name(), "ab");
    assert_eq!(Basis::Stored.name(), "stored");
}
