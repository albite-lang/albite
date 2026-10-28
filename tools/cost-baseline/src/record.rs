//! The committed cost record, `tools/tungsten1-check-cost.toml` (D2).
//!
//! Parse and render round-trip. Every `[bound]` field is required: a missing
//! one is an error, never a default, because a defaulted bound is a widening
//! nobody wrote.

use crate::{Bound, Environment, Samples};
use serde::{Deserialize, Serialize};

/// The record's path, relative to the workspace root.
pub const RECORD_PATH: &str = "tools/tungsten1-check-cost.toml";

const HEADER: &str = "\
# Cost record for `albite1 check src/compiler/main.tg` (ADR 15.10.26f).
# [measured] and [environment] are written by `make devcontainer-check-cost-bless`.
# [bound] is hand-edited, and only by an ADR (D6).
";

/// The record's `[measured]` table. The samples are stored, not only their
/// medians, so a later reader can see the spread the bound was set against.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Measured {
    pub commit: String,
    pub blessed_on: String,
    pub wall_s: Vec<f64>,
    pub max_rss_kb: Vec<u64>,
}

impl Measured {
    /// The stored arm, for `compare`: a bless refuses a failing run, so the
    /// stored samples carry no exit codes.
    #[must_use]
    pub fn samples(&self) -> Samples {
        Samples {
            wall_s: self.wall_s.clone(),
            max_rss_kb: self.max_rss_kb.clone(),
            exit_codes: Vec::new(),
        }
    }
}

/// The whole record, in file order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub bound: Bound,
    pub measured: Measured,
    pub environment: Environment,
}

/// What the gate found at the window's base (D2).
#[derive(Debug, Clone, PartialEq)]
pub enum RecordState {
    Present(Box<Record>),
    Absent,
    Malformed(String),
}

impl RecordState {
    /// Classify `git show <base>:<RECORD_PATH>`'s output: `None` is a file
    /// the base does not have.
    #[must_use]
    pub fn from_text(text: Option<&str>) -> Self {
        let Some(body) = text else {
            return RecordState::Absent;
        };
        parse(body).map_or_else(RecordState::Malformed, |record| {
            RecordState::Present(Box::new(record))
        })
    }

    /// The `record=` field of the verdict line, when there is one to print.
    #[must_use]
    pub fn label(&self) -> Option<&'static str> {
        match self {
            RecordState::Present(_) => None,
            RecordState::Absent => Some("absent"),
            RecordState::Malformed(_) => Some("malformed"),
        }
    }
}

/// Parse a record's text.
///
/// # Errors
/// A TOML error, a missing field or an unknown one, as a message.
pub fn parse(text: &str) -> Result<Record, String> {
    toml::from_str(text).map_err(|e| format!("{RECORD_PATH}: {e}"))
}

/// Render a record, with the header comment the bless writes.
///
/// # Errors
/// Only if `toml` cannot serialise the value, which a finite record never does.
pub fn render(record: &Record) -> Result<String, String> {
    let body = toml::to_string(record).map_err(|e| format!("render {RECORD_PATH}: {e}"))?;
    Ok(format!("{HEADER}{body}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record() -> Record {
        Record {
            bound: Bound {
                wall_pct: 10.0,
                rss_pct: 10.0,
                noise_pct: 8.0,
                drift_ceiling_pct: 20.0,
                stored_wall_margin_pct: 2.0,
                runs: 5,
                adr: "15.10.26f".into(),
            },
            measured: Measured {
                commit: "abc123".into(),
                blessed_on: "2026-10-16".into(),
                wall_s: vec![25.1, 25.2, 25.0, 25.3, 25.15],
                max_rss_kb: vec![1_985_624, 1_986_512, 1_985_932, 1_985_700, 1_986_000],
            },
            environment: Environment {
                kernel: "6.10.14-linuxkit".into(),
                os: "Ubuntu 24.04.1 LTS".into(),
                cpu_model: "0xd4f".into(),
                llc: "Ubuntu LLVM version 18.1.3".into(),
                rustc: "rustc 1.90.0".into(),
                nproc: 12,
                mem_total_kb: 16_000_000,
            },
        }
    }

    /// 15.10.26f AC3: the record round-trips through render and parse.
    #[test]
    fn record_round_trips_through_render_and_parse() {
        let record = sample_record();
        let text = render(&record).unwrap();
        assert!(text.starts_with("# Cost record for"), "{text}");
        assert_eq!(parse(&text).unwrap(), record);
    }

    /// 15.10.26f AC3: a missing `[bound]` field is an error, never a default.
    #[test]
    fn a_missing_bound_field_is_an_error() {
        let text = render(&sample_record()).unwrap();
        let without_noise = text
            .lines()
            .filter(|line| !line.starts_with("noise_pct"))
            .fold(String::new(), |acc, line| acc + line + "\n");
        let err = parse(&without_noise).unwrap_err();
        assert!(err.contains("noise_pct"), "{err}");
    }

    #[test]
    fn an_unknown_field_is_an_error() {
        let text = render(&sample_record())
            .unwrap()
            .replace("[bound]\n", "[bound]\nslack = 1\n");
        assert!(parse(&text).is_err());
    }

    #[test]
    fn record_state_classifies_absent_malformed_and_present() {
        assert_eq!(RecordState::from_text(None), RecordState::Absent);
        assert_eq!(RecordState::Absent.label(), Some("absent"));
        let malformed = RecordState::from_text(Some("[bound]\nwall_pct = \"x\"\n"));
        assert!(matches!(malformed, RecordState::Malformed(_)));
        assert_eq!(malformed.label(), Some("malformed"));
        let text = render(&sample_record()).unwrap();
        let present = RecordState::from_text(Some(&text));
        assert_eq!(present, RecordState::Present(Box::new(sample_record())));
        assert_eq!(present.label(), None);
    }

    #[test]
    fn measured_samples_carry_no_exit_codes() {
        let samples = sample_record().measured.samples();
        assert_eq!(samples.wall_s.len(), 5);
        assert_eq!(samples.max_rss_kb[0], 1_985_624);
        assert!(samples.exit_codes.is_empty());
        assert!(!samples.any_failed());
    }
}
