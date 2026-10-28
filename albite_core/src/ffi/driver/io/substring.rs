//! `substring`, the **length** reading (ADR 20.8.26c).
//!
//! This is the runtime counterpart of `Term::StrSubstring`, and the symbol the
//! self-hosted elaborator's CIR mirror names. Its third argument is a **count of
//! bytes**, not an end offset.
//!
//! Its sibling `tg_string_slice` in `strings.rs` takes an **end index**
//! instead. The two are one word apart and
//! the difference is invisible at every call site — which is the whole subject
//! of ADR 20.8.26c, where a `.tg` `substring` reading its third argument as an
//! end index shadowed a builtin reading it as a length, and returned `""` for
//! every identifier whose offset exceeded its own length. They live in separate
//! files, under separate names, for that reason.

use std::ptr;

use super::strings::TgString;

/// Whether `substring(bytes, start, len)` cuts inside a character (ADR
/// 16.10.26h D2): after the same clamp the export applies, a cut point `p`
/// (the clamped start, or the end) is inside a character when `p` is before
/// the end and the byte there is a UTF-8 continuation byte (`10xxxxxx`).
#[must_use]
pub fn cut_is_inside_character(bytes: &[u8], start: u64, len: u64) -> bool {
    let total = bytes.len() as u64;
    let start = start.min(total);
    // The evaluator's clamp (`byte_substring`): the end saturates, then clamps.
    let end = start.saturating_add(len).min(total);
    let inside = |point: u64| {
        bytes
            .get(point as usize)
            .is_some_and(|byte| byte & 0xC0 == 0x80)
    };
    inside(start) || inside(end)
}

/// Take `len` bytes of a Tungsten String starting at `start`.
///
/// A cut inside a character traps with code 13, as every other reader does
/// (ADR 16.10.26h D4) — no code generator emits this symbol today, so the
/// check keeps a future emitter from reopening the hole.
///
/// Borrows into the original buffer — no allocation, and the result must not be
/// freed separately. Both arguments are clamped, so an out-of-range `start`
/// yields the empty string rather than reading past the end.
///
/// # Safety
/// - `s.ptr` must be a valid pointer to at least `s.len` bytes
#[no_mangle]
pub extern "C" fn tg_string_substring(s: TgString, start: u64, len: u64) -> TgString {
    if s.ptr.is_null() {
        return TgString {
            ptr: ptr::null(),
            len: 0,
        };
    }
    // SAFETY: the caller guarantees `s.ptr` points at `s.len` bytes.
    let bytes = unsafe { std::slice::from_raw_parts(s.ptr.cast::<u8>(), s.len as usize) };
    if cut_is_inside_character(bytes, start, len) {
        crate::ffi::driver::console::tg_int_trap(
            crate::eval::IntTrapKind::SubstringInsideCharacter.code(),
        );
    }
    let start = start.min(s.len);
    let taken = len.min(s.len - start);
    TgString {
        ptr: unsafe { s.ptr.add(start as usize) },
        len: taken,
    }
}

#[cfg(test)]
mod tests {
    use super::{cut_is_inside_character, tg_string_substring, TgString};

    fn tg(s: &'static str) -> TgString {
        TgString {
            ptr: s.as_ptr().cast::<std::os::raw::c_char>(),
            len: s.len() as u64,
        }
    }

    fn read(s: TgString) -> String {
        if s.ptr.is_null() {
            return String::new();
        }
        let bytes = unsafe { std::slice::from_raw_parts(s.ptr.cast::<u8>(), s.len as usize) };
        String::from_utf8_lossy(bytes).into_owned()
    }

    /// The distinguishing case: a start offset greater than the requested
    /// length. An end-index reading returns `""` here; ADR 20.8.26c is about
    /// exactly this input.
    #[test]
    fn start_beyond_the_length_still_takes_len_bytes() {
        assert_eq!(read(tg_string_substring(tg("abcdefghij"), 6, 3)), "ghi");
    }

    #[test]
    fn a_zero_start_takes_a_prefix() {
        assert_eq!(read(tg_string_substring(tg("abcdefghij"), 0, 4)), "abcd");
    }

    #[test]
    fn a_zero_length_is_empty_rather_than_the_rest() {
        assert_eq!(read(tg_string_substring(tg("abcdefghij"), 2, 0)), "");
    }

    #[test]
    fn an_overlong_length_is_clamped_to_what_remains() {
        assert_eq!(read(tg_string_substring(tg("abc"), 1, 99)), "bc");
    }

    #[test]
    fn a_start_past_the_end_is_empty_rather_than_out_of_bounds() {
        let out = tg_string_substring(tg("abc"), 99, 3);
        assert_eq!(out.len, 0);
        assert_eq!(read(out), "");
    }

    // 16.10.26h AC 3: the three cuts inside `ï` (bytes 2..4) are inside a
    // character; a boundary cut and a clamped cut are not.
    #[test]
    fn cut_is_inside_character_on_either_cut_point_only() {
        let naive = "naïve café".as_bytes();
        assert!(cut_is_inside_character(naive, 0, 3), "end inside");
        assert!(cut_is_inside_character(naive, 3, 2), "start inside");
        assert!(cut_is_inside_character(naive, 3, 8), "both inside");
        assert!(!cut_is_inside_character(naive, 0, 4), "boundary");
        assert!(!cut_is_inside_character(naive, 2, 2), "the whole character");
        assert!(
            !cut_is_inside_character(b"ab", 1, u64::MAX),
            "clamped length"
        );
        assert!(!cut_is_inside_character(b"ab", 7, 1), "clamped start");
        // A cut at the very end reads nothing, even after a continuation byte.
        assert!(!cut_is_inside_character("é".as_bytes(), 0, 2));
        assert!(!cut_is_inside_character("é".as_bytes(), 2, 0));
    }

    #[test]
    fn a_null_subject_yields_the_null_empty_string() {
        let out = tg_string_substring(
            TgString {
                ptr: std::ptr::null(),
                len: 7,
            },
            0,
            3,
        );
        assert!(out.ptr.is_null());
        assert_eq!(out.len, 0);
    }
}
