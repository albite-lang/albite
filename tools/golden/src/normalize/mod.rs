//! Timing normalisation for golden snapshots (ADR 29.9.26i D9).
//!
//! `albite test --format json` records carry `"duration_ms":<n>`, wall time
//! that differs run to run. The human `finished in 0.00s` has passed goldens
//! only because trivial suites round to zero; JSON integers would not. So
//! every `"duration_ms":<digits>` becomes `"duration_ms":0`, and nothing else
//! is touched — the key's spelling is JSON-only, so human text never matches.
//! A subfolder because `src/` already holds the directory cap's eight files.

/// The key the rewrite anchors on, including serde_json's `:` with no space.
const DURATION_KEY: &str = "\"duration_ms\":";

/// Rewrite every `"duration_ms":<digits>` in `text` to `"duration_ms":0`.
/// Idempotent, and a key followed by anything but digits is left alone.
pub(crate) fn normalize_durations(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(DURATION_KEY) {
        let after_key = at + DURATION_KEY.len();
        normalized.push_str(&rest[..after_key]);
        let digits = rest[after_key..]
            .bytes()
            .take_while(u8::is_ascii_digit)
            .count();
        if digits > 0 {
            normalized.push('0');
        }
        rest = &rest[after_key + digits..];
    }
    normalized.push_str(rest);
    normalized
}

/// The attribute JUnit documents carry their seconds in (ADR 30.9.26k D9).
const JUNIT_TIME_ATTR: &str = "time=\"";

/// Rewrite every `time="<digits>.<digits>"` in `text` to `time="0.000"`, the
/// JUnit twin of [`normalize_durations`]. Human and JSON output never contain
/// `time="`, so no other golden can move. Idempotent; a value of any other
/// shape is left alone.
pub(crate) fn normalize_junit_times(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(JUNIT_TIME_ATTR) {
        let after_key = at + JUNIT_TIME_ATTR.len();
        normalized.push_str(&rest[..after_key]);
        rest = &rest[after_key..];
        if let Some(len) = seconds_value_len(rest) {
            normalized.push_str("0.000\"");
            rest = &rest[len..];
        }
    }
    normalized.push_str(rest);
    normalized
}

/// The byte length of a leading `<digits>.<digits>"`, closing quote
/// included, or `None` when `value` does not start with one.
fn seconds_value_len(value: &str) -> Option<usize> {
    let bytes = value.as_bytes();
    let whole = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
    if whole == 0 || bytes.get(whole) != Some(&b'.') {
        return None;
    }
    let fraction = bytes[whole + 1..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count();
    let quote = whole + 1 + fraction;
    (fraction > 0 && bytes.get(quote) == Some(&b'"')).then_some(quote + 1)
}

#[cfg(test)]
mod tests;
