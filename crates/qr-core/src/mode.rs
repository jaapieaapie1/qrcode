//! Data-encoding modes and their character-count-indicator lengths.

use crate::Version;

/// A QR data-encoding mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Digits `0-9`, 3 chars per 10 bits.
    Numeric,
    /// The 45-character alphanumeric set, 2 chars per 11 bits.
    Alphanumeric,
    /// Raw 8-bit bytes.
    Byte,
    /// Shift-JIS double-byte Kanji, 13 bits each.
    Kanji,
    /// Extended Channel Interpretation designator.
    Eci,
}

impl Mode {
    /// The 4-bit mode indicator placed in the bit stream (ISO Table 2).
    #[inline]
    pub fn indicator(self) -> u8 {
        match self {
            Mode::Numeric => 0b0001,
            Mode::Alphanumeric => 0b0010,
            Mode::Byte => 0b0100,
            Mode::Kanji => 0b1000,
            Mode::Eci => 0b0111,
        }
    }

    /// Length, in bits, of the character-count indicator for this mode at the
    /// given version (ISO Table 3). Returns `None` for [`Mode::Eci`], which has
    /// no character-count indicator.
    ///
    /// The three version groups are `1..=9`, `10..=26` and `27..=40`.
    pub fn char_count_bits(self, version: Version) -> Option<u8> {
        let group = match version.number() {
            1..=9 => 0,
            10..=26 => 1,
            _ => 2,
        };
        let bits = match self {
            Mode::Numeric => [10, 12, 14],
            Mode::Alphanumeric => [9, 11, 13],
            Mode::Byte => [8, 16, 16],
            Mode::Kanji => [8, 10, 12],
            Mode::Eci => return None,
        };
        Some(bits[group])
    }
}
