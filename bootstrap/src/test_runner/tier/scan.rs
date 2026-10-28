//! The two text predicates the manifest's guards are built on.
//!
//! Split from `tier/mod.rs` along the seam that means something: nothing here
//! knows what a manifest is. `calls_a_runtime_assertion` reads `.tg` source,
//! `glob_matches` reads a path key, and both are total functions over a string
//! — which is why every case below is asserted over a literal rather than
//! against the checkout.

/// The text guard (a) scans: the entry's source followed by the source of
/// every test companion in its module tree (ADR 2.10.26j D8). A companion's
/// tests run under the entry's tier, so its assertions are the entry's.
pub fn guarded_source(entry_source: &str, companion_sources: &[String]) -> String {
    let mut text = entry_source.to_string();
    for companion in companion_sources {
        text.push('\n');
        text.push_str(companion);
    }
    text
}

/// Whether `source` calls a runtime assertion helper — guard (a)'s subject.
///
/// Matches an identifier beginning `assert` immediately applied to arguments,
/// which is what every runtime assertion in the corpus looks like whatever
/// wrapper it goes through (`assert_eq`, `assert_ne`, `assert_eq_string`,
/// `assert`, `assert_some`). Deliberately NOT matched: a bare `use` of the
/// name, so importing an assertion without calling it does not force tier 5;
/// and `expect_type`/`expect_error`, which are elaboration-time assertions and
/// are exactly what a tier-3 file is made of.
///
/// A commented-out call still counts. That is the fail-safe direction: the
/// cost of a false positive is one file declared tier 5 and its tests actually
/// run, and the cost of a false negative is a silently skipped assertion.
pub fn calls_a_runtime_assertion(source: &str) -> bool {
    let bytes = source.as_bytes();
    source.match_indices("assert").any(|(start, _)| {
        let preceded_by_identifier_char = start
            .checked_sub(1)
            .is_some_and(|i| is_identifier_byte(bytes[i]));
        if preceded_by_identifier_char {
            return false;
        }
        // Length of the identifier starting here, measured on the tail slice so
        // the runs-to-end-of-input case needs no `len - start` arithmetic. That
        // subtraction had no observable effect either way — a correct index and
        // a too-large one both read past the end and yield `None` — which makes
        // it exactly the kind of untestable arithmetic worth not writing.
        let tail = &bytes[start..];
        let name_len = tail
            .iter()
            .position(|b| !is_identifier_byte(*b))
            .unwrap_or(tail.len());
        bytes.get(start + name_len) == Some(&b'(')
    })
}

fn is_identifier_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Whether the `/`-separated `key` matches `pattern`, where `*` stands for any
/// run of characters **within one path segment** (so `src/*/x.tg` does not
/// match `src/a/b/x.tg`). One `*` per segment; a second is matched literally.
pub(super) fn glob_matches(pattern: &str, key: &str) -> bool {
    let pattern_segments: Vec<&str> = pattern.split('/').collect();
    let key_segments: Vec<&str> = key.split('/').collect();
    pattern_segments.len() == key_segments.len()
        && pattern_segments
            .iter()
            .zip(&key_segments)
            .all(|(p, k)| segment_matches(p, k))
}

fn segment_matches(pattern: &str, segment: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == segment,
        Some((prefix, suffix)) => {
            segment.len() >= prefix.len() + suffix.len()
                && segment.starts_with(prefix)
                && segment.ends_with(suffix)
        }
    }
}

// Tests: scan_tests.rs
#[cfg(test)]
#[path = "scan_tests.rs"]
mod tests;
