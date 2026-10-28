//! Which source text a diagnostic renders against, and how a byte offset in
//! it becomes a line number.
//!
//! Split out of `renderers/mod.rs` for the 400-LOC file limit (ADR 2.10.26j
//! D4), unchanged except that [`byte_offset_to_line`] is now bounds-safe.

use crate::driver::modules::SourceMap;
use std::path::Path;

/// Convert a byte offset to a 1-based line number using the source map.
/// Falls back to the raw byte offset if the source is unavailable — or if the
/// offset lies past its end (ADR 2.10.26j D4's backstop): an unrelocated
/// companion span measured against its sibling's text used to slice out of
/// bounds and panic, where a wrong number is the better failure.
pub(super) fn byte_offset_to_line(
    source_map: Option<&SourceMap>,
    file_path: &Path,
    offset: u32,
) -> String {
    let prefix = source_map
        .and_then(|sm| sm.get(file_path))
        .and_then(|source| source.get(..offset as usize));
    match prefix {
        Some(prefix) => (prefix.bytes().filter(|&b| b == b'\n').count() + 1).to_string(),
        None => offset.to_string(),
    }
}

/// Get the appropriate source and filename for an error.
pub(in crate::driver::diagnostics) fn get_error_source<'a>(
    default_source: &'a str,
    default_filename: &str,
    source_map: &'a SourceMap,
    file_path: Option<&Path>,
) -> (&'a str, String) {
    if let Some(path) = file_path {
        if let Some(source) = source_map.get(path) {
            return (source, path.display().to_string());
        }
    }
    (default_source, default_filename.to_string())
}
