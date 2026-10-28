//! The lead table and its walk agree with `std::str::from_utf8` (ADR 3.10.26n
//! D3: UTF-8 is checked at conversion, on both sides, by the same rule).

use super::{utf8_lead_entry, utf8_valid_by_lead_table, UTF8_LEAD_INVALID};

/// A macro rather than a helper fn: the mutation engine empties a helper's
/// body, which would leave every test below asserting nothing.
macro_rules! agrees_with_std {
    ($bytes:expr) => {{
        let bytes: &[u8] = $bytes;
        assert_eq!(
            utf8_valid_by_lead_table(bytes),
            std::str::from_utf8(bytes).is_ok(),
            "table walk and std disagree on {bytes:02X?}"
        );
    }};
}

/// Every one- and two-byte sequence, exhaustively.
#[test]
fn every_one_and_two_byte_sequence_agrees_with_std() {
    for first in 0..=u8::MAX {
        agrees_with_std!(&[first]);
        for second in 0..=u8::MAX {
            agrees_with_std!(&[first, second]);
        }
    }
}

/// Every three-byte sequence whose lead begins a three-byte scalar, with every
/// second byte and a sample of third bytes spanning each class boundary.
#[test]
fn three_byte_sequences_agree_with_std_at_every_boundary() {
    let thirds = [0x00, 0x7F, 0x80, 0x9F, 0xA0, 0xBF, 0xC0, 0xFF];
    for lead in 0xE0..=0xEF_u8 {
        for second in 0..=u8::MAX {
            for third in thirds {
                agrees_with_std!(&[lead, second, third]);
            }
        }
    }
}

/// Four-byte scalars at the edges `F0` (overlong) and `F4` (above U+10FFFF)
/// narrow, with every second byte, plus a short tail.
#[test]
fn four_byte_sequences_agree_with_std_at_every_boundary() {
    let tails = [0x00, 0x7F, 0x80, 0xBF, 0xC0];
    for lead in 0xF0..=0xF7_u8 {
        for second in 0..=u8::MAX {
            for third in tails {
                for fourth in tails {
                    agrees_with_std!(&[lead, second, third, fourth]);
                }
            }
            agrees_with_std!(&[lead, second, 0x80]);
        }
    }
}

/// Mixed scalars, truncation and the empty buffer.
#[test]
fn mixed_text_and_truncations_agree_with_std() {
    let text = "aé€𝄞z".as_bytes();
    agrees_with_std!(text);
    agrees_with_std!(&[]);
    for cut in 0..text.len() {
        agrees_with_std!(&text[..cut]);
        agrees_with_std!(&text[cut..]);
    }
}

/// The entries the narrowed ranges hinge on, pinned by value.
#[test]
fn lead_entries_pack_count_and_first_range() {
    assert_eq!(utf8_lead_entry(b'A'), 0);
    assert_eq!(utf8_lead_entry(0xC2), (1 << 16) | (0x80 << 8) | 0xBF);
    assert_eq!(utf8_lead_entry(0xE0), (2 << 16) | (0xA0 << 8) | 0xBF);
    assert_eq!(utf8_lead_entry(0xED), (2 << 16) | (0x80 << 8) | 0x9F);
    assert_eq!(utf8_lead_entry(0xF0), (3 << 16) | (0x90 << 8) | 0xBF);
    assert_eq!(utf8_lead_entry(0xF4), (3 << 16) | (0x80 << 8) | 0x8F);
    for invalid in [0x80, 0xBF, 0xC0, 0xC1, 0xF5, 0xFF] {
        assert_eq!(utf8_lead_entry(invalid), UTF8_LEAD_INVALID, "{invalid:02X}");
    }
}
