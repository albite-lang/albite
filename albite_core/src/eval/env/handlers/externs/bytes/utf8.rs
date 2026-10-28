//! The UTF-8 lead-byte table native `tg_bytes_into_string` validates with.
//!
//! The evaluator checks UTF-8 with `std::str::from_utf8`. Native code may call
//! no Rust (ADR 3.10.26n D4), so the code generator emits a 256-entry constant
//! built from [`utf8_lead_entry`] and walks the buffer with it. The walk is
//! written out here as [`utf8_valid_by_lead_table`], step for step the loop the
//! code generator emits, so the table and the algorithm are both checked
//! against the standard library in an LLVM-free crate — the parity `diff exec`
//! then checks end to end.

/// The entry for a byte that cannot begin a scalar: a continuation byte, an
/// overlong lead (`C0`, `C1`) or a lead above `F4`.
pub const UTF8_LEAD_INVALID: u32 = u32::MAX;

/// The packed table entry for lead byte `lead`.
///
/// `(continuations << 16) | (second_lo << 8) | second_hi`: how many
/// continuation bytes follow, and the inclusive range the **first** of them
/// must fall in. Later continuations are always `80..=BF`. The narrowed first
/// ranges are what reject overlong forms (`E0`, `F0`), surrogates (`ED`) and
/// scalars above `U+10FFFF` (`F4`) — the cases RFC 3629 §4 tabulates. ASCII
/// is `0`: no continuation.
#[must_use]
pub fn utf8_lead_entry(lead: u8) -> u32 {
    // `+`, not `|`: the three fields never overlap, so the two agree, and `|`
    // would make every `| → ^` mutant equivalent and unkillable.
    let packed = |continuations: u32, lo: u32, hi: u32| (continuations << 16) + (lo << 8) + hi;
    match lead {
        0x00..=0x7F => 0,
        0xC2..=0xDF => packed(1, 0x80, 0xBF),
        0xE0 => packed(2, 0xA0, 0xBF),
        0xE1..=0xEC | 0xEE..=0xEF => packed(2, 0x80, 0xBF),
        0xED => packed(2, 0x80, 0x9F),
        0xF0 => packed(3, 0x90, 0xBF),
        0xF1..=0xF3 => packed(3, 0x80, 0xBF),
        0xF4 => packed(3, 0x80, 0x8F),
        _ => UTF8_LEAD_INVALID,
    }
}

/// Validate `bytes` exactly as the emitted loop does.
///
/// Per scalar: read the lead's entry and refuse an invalid one; refuse when
/// fewer than `continuations` bytes remain (`continuations < len − i`,
/// unsigned, so it cannot wrap); check the first continuation against its
/// narrowed range and the rest against `80..=BF`; advance by
/// `continuations + 1`.
#[must_use]
pub fn utf8_valid_by_lead_table(bytes: &[u8]) -> bool {
    let len = bytes.len();
    let mut index = 0;
    while index < len {
        let entry = utf8_lead_entry(bytes[index]);
        if entry == UTF8_LEAD_INVALID {
            return false;
        }
        let continuations = (entry >> 16) as usize;
        let (lo, hi) = (((entry >> 8) & 0xFF) as u8, (entry & 0xFF) as u8);
        if continuations >= len - index {
            return false;
        }
        if continuations >= 1 && !(lo..=hi).contains(&bytes[index + 1]) {
            return false;
        }
        // Continuations two and three, each `80..=BF` (`0b10xx_xxxx`).
        if (2..=continuations).any(|offset| bytes[index + offset] & 0xC0 != 0x80) {
            return false;
        }
        index += continuations + 1;
    }
    true
}

// Tests: utf8_tests.rs — the table and its walk against `std::str::from_utf8`.
#[cfg(test)]
#[path = "utf8_tests.rs"]
mod tests;
