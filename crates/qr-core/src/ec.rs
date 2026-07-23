//! Error-correction levels.

/// The four QR error-correction levels, in ascending order of redundancy.
///
/// The order of the variants is *not* the bit encoding used in the format
/// information — use [`EcLevel::format_bits`] for that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EcLevel {
    /// ~7% of codewords recoverable.
    L,
    /// ~15% of codewords recoverable.
    M,
    /// ~25% of codewords recoverable.
    Q,
    /// ~30% of codewords recoverable.
    H,
}

impl EcLevel {
    /// All four levels, in `L, M, Q, H` order.
    pub const ALL: [EcLevel; 4] = [EcLevel::L, EcLevel::M, EcLevel::Q, EcLevel::H];

    /// Dense index `0..4` for table lookups (`L=0, M=1, Q=2, H=3`).
    #[inline]
    pub fn index(self) -> usize {
        match self {
            EcLevel::L => 0,
            EcLevel::M => 1,
            EcLevel::Q => 2,
            EcLevel::H => 3,
        }
    }

    /// The 2-bit value used in the 15-bit format information (ISO Table 12):
    /// `M=00, L=01, H=10, Q=11`.
    #[inline]
    pub fn format_bits(self) -> u8 {
        match self {
            EcLevel::M => 0b00,
            EcLevel::L => 0b01,
            EcLevel::H => 0b10,
            EcLevel::Q => 0b11,
        }
    }

    /// Inverse of [`EcLevel::format_bits`].
    #[inline]
    pub fn from_format_bits(bits: u8) -> Option<EcLevel> {
        match bits & 0b11 {
            0b00 => Some(EcLevel::M),
            0b01 => Some(EcLevel::L),
            0b10 => Some(EcLevel::H),
            0b11 => Some(EcLevel::Q),
            _ => None,
        }
    }
}
