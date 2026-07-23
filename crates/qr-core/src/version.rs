//! QR symbol versions (1..=40) and their geometric properties.

use crate::Error;

/// Total number of codewords (data + error correction) per version, indexed by
/// `version - 1`. From ISO/IEC 18004 Table 1.
const TOTAL_CODEWORDS: [u16; 40] = [
    26, 44, 70, 100, 134, 172, 196, 242, 292, 346, 404, 466, 532, 581, 655, 733, 815, 901, 991,
    1085, 1156, 1258, 1364, 1474, 1588, 1706, 1828, 1921, 2051, 2185, 2323, 2465, 2611, 2761, 2876,
    3034, 3196, 3362, 3532, 3706,
];

/// A QR symbol version in the range `1..=40`.
///
/// The module grid is square with side length [`Version::size`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(u8);

impl Version {
    /// Creates a version, validating that `v` is in `1..=40`.
    #[inline]
    pub fn new(v: u8) -> Result<Version, Error> {
        if (1..=40).contains(&v) {
            Ok(Version(v))
        } else {
            Err(Error::InvalidVersion)
        }
    }

    /// The version number (`1..=40`).
    #[inline]
    pub fn number(self) -> u8 {
        self.0
    }

    /// Side length of the module grid: `17 + 4 * version`.
    #[inline]
    pub fn size(self) -> u32 {
        17 + 4 * self.0 as u32
    }

    /// Total number of codewords (data + EC) the symbol holds.
    #[inline]
    pub fn total_codewords(self) -> u16 {
        TOTAL_CODEWORDS[self.0 as usize - 1]
    }

    /// Number of "remainder" bits left over after the data region is filled
    /// with codewords (ISO Table 1). These bits are always zero.
    #[inline]
    pub fn remainder_bits(self) -> u8 {
        match self.0 {
            1 => 0,
            2..=6 => 7,
            7..=13 => 0,
            14..=20 => 3,
            21..=27 => 4,
            28..=34 => 3,
            _ => 0, // 35..=40
        }
    }

    /// Center coordinates of the alignment-pattern grid for this version
    /// (ISO Annex E). Alignment patterns are placed at every `(col, row)`
    /// combination of these coordinates except the three that collide with the
    /// finder patterns. Version 1 has no alignment patterns (empty slice).
    pub fn alignment_pattern_positions(self) -> &'static [u8] {
        ALIGNMENT[self.0 as usize - 1]
    }
}

/// Alignment-pattern center coordinate lists, indexed by `version - 1`.
const ALIGNMENT: [&[u8]; 40] = [
    &[],
    &[6, 18],
    &[6, 22],
    &[6, 26],
    &[6, 30],
    &[6, 34],
    &[6, 22, 38],
    &[6, 24, 42],
    &[6, 26, 46],
    &[6, 28, 50],
    &[6, 30, 54],
    &[6, 32, 58],
    &[6, 34, 62],
    &[6, 26, 46, 66],
    &[6, 26, 48, 70],
    &[6, 26, 50, 74],
    &[6, 30, 54, 78],
    &[6, 30, 56, 82],
    &[6, 30, 58, 86],
    &[6, 34, 62, 90],
    &[6, 28, 50, 72, 94],
    &[6, 26, 50, 74, 98],
    &[6, 30, 54, 78, 102],
    &[6, 28, 54, 80, 106],
    &[6, 32, 58, 84, 110],
    &[6, 30, 58, 86, 114],
    &[6, 34, 62, 90, 118],
    &[6, 26, 50, 74, 98, 122],
    &[6, 30, 54, 78, 102, 126],
    &[6, 26, 52, 78, 104, 130],
    &[6, 30, 56, 82, 108, 134],
    &[6, 34, 60, 86, 112, 138],
    &[6, 30, 58, 86, 114, 142],
    &[6, 34, 62, 90, 118, 146],
    &[6, 30, 54, 78, 102, 126, 150],
    &[6, 24, 50, 76, 102, 128, 154],
    &[6, 28, 54, 80, 106, 132, 158],
    &[6, 32, 58, 84, 110, 136, 162],
    &[6, 26, 54, 82, 110, 138, 166],
    &[6, 30, 58, 86, 114, 142, 170],
];
