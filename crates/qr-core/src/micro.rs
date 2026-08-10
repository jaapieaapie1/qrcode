//! Micro QR (versions M1-M4) primitives, per ISO/IEC 18004:2024.
//!
//! This module is gated behind the `micro` cargo feature (on by default) and is
//! deliberately *parallel* to the full-QR primitives rather than woven into
//! them: [`MicroVersion`] sits beside [`Version`](crate::Version),
//! [`MicroEcLevel`] beside [`EcLevel`], [`micro_is_function_module`] beside
//! [`is_function_module`](crate::is_function_module). Nothing here changes the
//! meaning of an existing type, so a build with the feature off is byte-for-byte
//! the old crate.
//!
//! What is *not* duplicated: GF(256), Reed-Solomon, the `(15, 5)` BCH code and
//! [`BitMatrix`](crate::BitMatrix) are shared verbatim with the QR path. ISO
//! Annex C.2 specifies the same BCH code for both symbologies, differing only in
//! the XOR mask, and ISO 7.5 specifies the same Reed-Solomon construction.
//!
//! # Coordinates
//!
//! As everywhere in this crate, coordinates are `(x, y)` = (column, row) with
//! the origin at the top-left corner of the symbol.
//!
//! # Layout summary
//!
//! A micro symbol is `9 + 2n` modules square (11, 13, 15, 17 for M1..M4;
//! ISO 5.3.2.2). It carries a *single* finder pattern in the top-left corner
//! with its separator (ISO 5.3.3.2), a horizontal timing pattern along the whole
//! of row 0 and a vertical timing pattern along the whole of column 0
//! (ISO 5.3.5), and a single 15-bit format information strip wrapped around the
//! finder (ISO 7.9.2, Figure 26). There are no alignment patterns, no version
//! information and no fixed dark module.

use crate::{BitMatrix, EcLevel, bch, mask};

/// XOR mask applied to the 15-bit micro format information: `100010001000101`
/// (ISO 7.9.2 and Annex C.2). Note this differs from the QR mask `0x5412`.
pub const MICRO_FORMAT_MASK: u16 = 0x4445;

/// Maps a micro data-mask reference (`0..=3`) onto the equivalent QR data-mask
/// reference, so the shared [`mask_bit`](crate::mask_bit) predicates can be
/// reused (ISO Table 10, which lists both references against one condition).
///
/// Micro `00` is QR `001`, micro `01` is QR `100`, micro `10` is QR `110` and
/// micro `11` is QR `111`.
pub const MICRO_MASK_TO_QR: [u8; 4] = [1, 4, 6, 7];

/// The `(x, y)` position of each micro format-information module, indexed by bit
/// number: entry `0` holds the least significant bit, entry `14` the most
/// significant (ISO 7.9.2, Figure 26).
///
/// Bits 0..=7 run down column 8 from row 1 to row 8; bits 8..=14 continue
/// leftwards along row 8 from column 7 to column 1. The positions are identical
/// for every micro version.
pub const MICRO_FORMAT_INFO_POSITIONS: [(u32, u32); 15] = [
    (8, 1),
    (8, 2),
    (8, 3),
    (8, 4),
    (8, 5),
    (8, 6),
    (8, 7),
    (8, 8),
    (7, 8),
    (6, 8),
    (5, 8),
    (4, 8),
    (3, 8),
    (2, 8),
    (1, 8),
];

/// A micro QR symbol version, M1 through M4 (ISO 5.3.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MicroVersion {
    /// 11 x 11 modules. Numeric mode only, error detection only.
    M1,
    /// 13 x 13 modules. Numeric and alphanumeric modes.
    M2,
    /// 15 x 15 modules. All four modes.
    M3,
    /// 17 x 17 modules. All four modes; the only version offering level Q.
    M4,
}

impl MicroVersion {
    /// All four versions, in ascending order.
    pub const ALL: [MicroVersion; 4] = [
        MicroVersion::M1,
        MicroVersion::M2,
        MicroVersion::M3,
        MicroVersion::M4,
    ];

    /// The version number `1..=4` (the `n` in `Mn`).
    #[inline]
    pub fn number(self) -> u8 {
        match self {
            MicroVersion::M1 => 1,
            MicroVersion::M2 => 2,
            MicroVersion::M3 => 3,
            MicroVersion::M4 => 4,
        }
    }

    /// Builds a version from its number `1..=4`, or `None` if out of range.
    #[inline]
    pub fn from_number(n: u8) -> Option<MicroVersion> {
        match n {
            1 => Some(MicroVersion::M1),
            2 => Some(MicroVersion::M2),
            3 => Some(MicroVersion::M3),
            4 => Some(MicroVersion::M4),
            _ => None,
        }
    }

    /// The side length in modules: `9 + 2 * number()`, i.e. 11, 13, 15, 17
    /// (ISO 5.3.2.2, Table 1 column A).
    #[inline]
    pub fn size(self) -> u32 {
        9 + 2 * self.number() as u32
    }

    /// Recovers a version from a symbol side length, or `None` if the size is
    /// not one of 11, 13, 15, 17.
    #[inline]
    pub fn from_size(size: u32) -> Option<MicroVersion> {
        match size {
            11 => Some(MicroVersion::M1),
            13 => Some(MicroVersion::M2),
            15 => Some(MicroVersion::M3),
            17 => Some(MicroVersion::M4),
            _ => None,
        }
    }

    /// Total number of codewords in the symbol, data plus error correction
    /// (ISO Table 1 column E, ISO Table 9): 5, 10, 17, 24.
    ///
    /// In M1 and M3 one of these "codewords" is only 4 bits long — see
    /// [`MicroSymbolInfo::final_data_codeword_bits`].
    #[inline]
    pub fn total_codewords(self) -> u8 {
        match self {
            MicroVersion::M1 => 5,
            MicroVersion::M2 => 10,
            MicroVersion::M3 => 17,
            MicroVersion::M4 => 24,
        }
    }

    /// Number of modules available to data and error correction codewords, i.e.
    /// everything that is not a function pattern or format information
    /// (ISO Table 1 column D): 36, 80, 132, 192.
    ///
    /// Micro symbols have no remainder bits (ISO Table 1), so this is always
    /// exactly the bit length of the codeword stream.
    #[inline]
    pub fn data_modules(self) -> u32 {
        match self {
            MicroVersion::M1 => 36,
            MicroVersion::M2 => 80,
            MicroVersion::M3 => 132,
            MicroVersion::M4 => 192,
        }
    }

    /// Bit width of the mode indicator for this version (ISO Table 2): 0, 1, 2,
    /// 3 bits for M1..M4. M1 carries no mode indicator at all because numeric is
    /// its only mode.
    #[inline]
    pub fn mode_indicator_bits(self) -> u8 {
        self.number() - 1
    }

    /// Bit length of the terminator for this version (ISO Table 2, ISO 7.4.10):
    /// 3, 5, 7, 9 zero bits for M1..M4.
    ///
    /// The terminator is omitted, or abbreviated to the space that remains, when
    /// the data bit stream leaves less room than this.
    #[inline]
    pub fn terminator_bits(self) -> u8 {
        2 * self.number() + 1
    }

    /// The error-correction selections available for this version, in ascending
    /// order of redundancy (ISO Table 13).
    ///
    /// M1 offers only [`MicroEcLevel::Detection`]; M2 and M3 offer L and M; M4
    /// adds Q. Level H is never available in a micro symbol (ISO 7.5.1).
    #[inline]
    pub fn ec_levels(self) -> &'static [MicroEcLevel] {
        match self {
            MicroVersion::M1 => &[MicroEcLevel::Detection],
            MicroVersion::M2 | MicroVersion::M3 => &[MicroEcLevel::L, MicroEcLevel::M],
            MicroVersion::M4 => &[MicroEcLevel::L, MicroEcLevel::M, MicroEcLevel::Q],
        }
    }
}

/// The error-correction selection of a micro symbol (ISO Table 13).
///
/// This is deliberately *not* [`EcLevel`]: version M1 has no error correction
/// level at all, only two error *detection* codewords, and level H is never
/// available. Use [`MicroEcLevel::ec_level`] and
/// [`MicroEcLevel::from_ec_level`] to move between the two types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MicroEcLevel {
    /// Version M1 only: two codewords of error *detection*, no correction
    /// capability (ISO Table 9 gives M1 the block structure `(5, 3, 0)` — a
    /// correction capacity of zero).
    Detection,
    /// ~7% recovery.
    L,
    /// ~15% recovery.
    M,
    /// ~25% recovery. Version M4 only.
    Q,
}

impl MicroEcLevel {
    /// All four selections, in ascending order of redundancy.
    pub const ALL: [MicroEcLevel; 4] = [
        MicroEcLevel::Detection,
        MicroEcLevel::L,
        MicroEcLevel::M,
        MicroEcLevel::Q,
    ];

    /// The equivalent [`EcLevel`], or `None` for
    /// [`MicroEcLevel::Detection`] which has no QR counterpart.
    #[inline]
    pub fn ec_level(self) -> Option<EcLevel> {
        match self {
            MicroEcLevel::Detection => None,
            MicroEcLevel::L => Some(EcLevel::L),
            MicroEcLevel::M => Some(EcLevel::M),
            MicroEcLevel::Q => Some(EcLevel::Q),
        }
    }

    /// The micro selection matching an [`EcLevel`], or `None` for
    /// [`EcLevel::H`], which no micro version supports (ISO 7.5.1).
    #[inline]
    pub fn from_ec_level(ec: EcLevel) -> Option<MicroEcLevel> {
        match ec {
            EcLevel::L => Some(MicroEcLevel::L),
            EcLevel::M => Some(MicroEcLevel::M),
            EcLevel::Q => Some(MicroEcLevel::Q),
            EcLevel::H => None,
        }
    }
}

/// The capacity and error-correction characteristics of one micro symbol,
/// identified by its `(version, ec)` pair.
///
/// Collates ISO Table 1 (module counts), Table 7 (data capacity), Table 9
/// (error correction structure) and Table 13 (symbol numbers). Every micro
/// symbol is a single Reed-Solomon block (note under ISO 7.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MicroSymbolInfo {
    /// The symbol version.
    pub version: MicroVersion,
    /// The error-correction selection.
    pub ec: MicroEcLevel,
    /// The 3-bit symbol number `0..=7` written into the format information
    /// (ISO Table 13).
    pub symbol_number: u8,
    /// Total codewords, data plus error correction (ISO Table 9, `c`).
    pub total_codewords: u8,
    /// Data codewords, counting the 4-bit final codeword of M1/M3 as one
    /// (ISO Table 7 and Table 9, `k`).
    pub data_codewords: u8,
    /// Error correction codewords (ISO Table 9).
    pub ec_codewords: u8,
    /// Usable data bits, i.e. the bits available for mode indicators, character
    /// counts, data and the terminator (ISO Table 7).
    pub data_bits: u16,
    /// Number of codeword errors the block can correct (ISO Table 9, `r`).
    /// Zero for M1, whose error correction codewords only detect.
    pub error_correction_capacity: u8,
    /// Misdecode protection codewords `p` (ISO 7.5.1). Where `p > 0`, `p` of the
    /// error correction codewords detect rather than correct, which is why
    /// [`MicroSymbolInfo::error_correction_capacity`] can be below
    /// `ec_codewords / 2`.
    pub misdecode_protection: u8,
}

impl MicroSymbolInfo {
    /// Bit length of the final data codeword: 4 for M1 and M3, 8 elsewhere
    /// (ISO 7.4.11).
    ///
    /// All other codewords, including every error correction codeword, are 8
    /// bits. ISO 7.7.3 notes that this half codeword is laid out as a 2 x 2
    /// square block of modules, which falls out naturally from the placement
    /// order produced by [`micro_data_modules`].
    #[inline]
    pub fn final_data_codeword_bits(&self) -> u8 {
        match self.version {
            MicroVersion::M1 | MicroVersion::M3 => 4,
            MicroVersion::M2 | MicroVersion::M4 => 8,
        }
    }

    /// Whether the final data codeword is a 4-bit half codeword (M1 and M3).
    #[inline]
    pub fn has_half_final_data_codeword(&self) -> bool {
        self.final_data_codeword_bits() == 4
    }
}

/// The eight valid micro symbols, indexed by ISO Table 13 symbol number.
///
/// Columns: version, ec, total codewords (Table 9 `c`), data codewords
/// (Table 7 / Table 9 `k`), ec codewords (Table 9), data bits (Table 7),
/// correction capacity (Table 9 `r`), misdecode protection `p` (ISO 7.5.1).
const MICRO_SYMBOLS: [MicroSymbolInfo; 8] = [
    MicroSymbolInfo {
        version: MicroVersion::M1,
        ec: MicroEcLevel::Detection,
        symbol_number: 0,
        total_codewords: 5,
        data_codewords: 3,
        ec_codewords: 2,
        data_bits: 20,
        error_correction_capacity: 0,
        misdecode_protection: 2,
    },
    MicroSymbolInfo {
        version: MicroVersion::M2,
        ec: MicroEcLevel::L,
        symbol_number: 1,
        total_codewords: 10,
        data_codewords: 5,
        ec_codewords: 5,
        data_bits: 40,
        error_correction_capacity: 1,
        misdecode_protection: 3,
    },
    MicroSymbolInfo {
        version: MicroVersion::M2,
        ec: MicroEcLevel::M,
        symbol_number: 2,
        total_codewords: 10,
        data_codewords: 4,
        ec_codewords: 6,
        data_bits: 32,
        error_correction_capacity: 2,
        misdecode_protection: 2,
    },
    MicroSymbolInfo {
        version: MicroVersion::M3,
        ec: MicroEcLevel::L,
        symbol_number: 3,
        total_codewords: 17,
        data_codewords: 11,
        ec_codewords: 6,
        data_bits: 84,
        error_correction_capacity: 2,
        misdecode_protection: 2,
    },
    MicroSymbolInfo {
        version: MicroVersion::M3,
        ec: MicroEcLevel::M,
        symbol_number: 4,
        total_codewords: 17,
        data_codewords: 9,
        ec_codewords: 8,
        data_bits: 68,
        error_correction_capacity: 4,
        misdecode_protection: 0,
    },
    MicroSymbolInfo {
        version: MicroVersion::M4,
        ec: MicroEcLevel::L,
        symbol_number: 5,
        total_codewords: 24,
        data_codewords: 16,
        ec_codewords: 8,
        data_bits: 128,
        error_correction_capacity: 3,
        misdecode_protection: 2,
    },
    MicroSymbolInfo {
        version: MicroVersion::M4,
        ec: MicroEcLevel::M,
        symbol_number: 6,
        total_codewords: 24,
        data_codewords: 14,
        ec_codewords: 10,
        data_bits: 112,
        error_correction_capacity: 5,
        misdecode_protection: 0,
    },
    MicroSymbolInfo {
        version: MicroVersion::M4,
        ec: MicroEcLevel::Q,
        symbol_number: 7,
        total_codewords: 24,
        data_codewords: 10,
        ec_codewords: 14,
        data_bits: 80,
        error_correction_capacity: 7,
        misdecode_protection: 0,
    },
];

/// Looks up the characteristics of a micro symbol, or `None` if the version does
/// not offer that error-correction selection (ISO Table 13).
pub fn micro_symbol_info(version: MicroVersion, ec: MicroEcLevel) -> Option<MicroSymbolInfo> {
    MICRO_SYMBOLS
        .iter()
        .find(|info| info.version == version && info.ec == ec)
        .copied()
}

/// Looks up a micro symbol by its ISO Table 13 symbol number, or `None` if the
/// number is outside `0..=7`.
#[inline]
pub fn micro_symbol_info_from_number(symbol_number: u8) -> Option<MicroSymbolInfo> {
    MICRO_SYMBOLS.get(symbol_number as usize).copied()
}

/// Every valid micro symbol, in symbol-number order.
#[inline]
pub fn micro_symbols() -> &'static [MicroSymbolInfo; 8] {
    &MICRO_SYMBOLS
}

/// A micro data-encoding mode (ISO Table 2).
///
/// This is a parallel type to [`Mode`](crate::Mode): micro symbols have no ECI,
/// no structured append and no FNC1 (ISO 7.4.3, 7.4.9), and the mode indicator
/// values and widths differ per version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MicroMode {
    /// Digits `0-9`, 3 digits per 10 bits.
    Numeric,
    /// The 45-character alphanumeric set, 2 characters per 11 bits.
    Alphanumeric,
    /// Raw 8-bit bytes.
    Byte,
    /// Shift-JIS double-byte Kanji, 13 bits each.
    Kanji,
}

impl MicroMode {
    /// All four modes, in mode-indicator order.
    pub const ALL: [MicroMode; 4] = [
        MicroMode::Numeric,
        MicroMode::Alphanumeric,
        MicroMode::Byte,
        MicroMode::Kanji,
    ];

    /// Dense index `0..4` (`Numeric=0, Alphanumeric=1, Byte=2, Kanji=3`), which
    /// is also the mode indicator *value* wherever the mode is available.
    #[inline]
    fn index(self) -> usize {
        match self {
            MicroMode::Numeric => 0,
            MicroMode::Alphanumeric => 1,
            MicroMode::Byte => 2,
            MicroMode::Kanji => 3,
        }
    }

    /// Whether this mode may be used in the given version (ISO Table 2).
    ///
    /// M1 is numeric only; M2 adds alphanumeric; M3 and M4 add byte and Kanji.
    #[inline]
    pub fn is_available(self, version: MicroVersion) -> bool {
        let available = match version {
            MicroVersion::M1 => 1,
            MicroVersion::M2 => 2,
            MicroVersion::M3 | MicroVersion::M4 => 4,
        };
        self.index() < available
    }

    /// The mode indicator as `(value, bit width)`, or `None` if the mode is not
    /// available in this version (ISO Table 2).
    ///
    /// The width is [`MicroVersion::mode_indicator_bits`], so M1 returns
    /// `(0, 0)`: a zero-width indicator that contributes no bits at all.
    #[inline]
    pub fn indicator(self, version: MicroVersion) -> Option<(u8, u8)> {
        if !self.is_available(version) {
            return None;
        }
        Some((self.index() as u8, version.mode_indicator_bits()))
    }

    /// Bit width of the character count indicator for this mode and version
    /// (ISO Table 3), or `None` if the mode is not available in this version.
    #[inline]
    pub fn char_count_bits(self, version: MicroVersion) -> Option<u8> {
        if !self.is_available(version) {
            return None;
        }
        // Rows are versions M1..M4, columns are numeric/alnum/byte/kanji.
        const BITS: [[u8; 4]; 4] = [[3, 0, 0, 0], [4, 3, 0, 0], [5, 4, 4, 3], [6, 5, 5, 4]];
        Some(BITS[version.number() as usize - 1][self.index()])
    }
}

/// Returns whether the micro data mask numbered `mask_id` (`0..=3`) inverts the
/// module at column `x`, row `y` (ISO Table 10).
///
/// The four micro masks are a subset of the eight QR masks; this forwards to
/// [`mask_bit`](crate::mask_bit) through [`MICRO_MASK_TO_QR`]. Mask ids outside
/// `0..=3` return `false`.
#[inline]
pub fn micro_mask_bit(mask_id: u8, x: u32, y: u32) -> bool {
    match MICRO_MASK_TO_QR.get(mask_id as usize) {
        Some(&qr_id) => mask::mask_bit(qr_id, x, y),
        None => false,
    }
}

/// Encodes the 15-bit micro format information for a symbol number (`0..=7`) and
/// micro data mask reference (`0..=3`), including the XOR mask
/// [`MICRO_FORMAT_MASK`] (ISO 7.9.2, Annex C.2).
///
/// The 5 data bits are the 3-bit symbol number followed by the 2-bit mask
/// reference. Values outside range are masked down rather than rejected, so this
/// is total; use [`micro_format_info_encode`] for the checked form.
fn micro_format_info_encode_raw(symbol_number: u8, mask: u8) -> u16 {
    let data = (((symbol_number & 0x7) as u32) << 2) | ((mask & 0x3) as u32);
    let rem = bch::bch_remainder(data << 10, bch::FORMAT_GEN_POLY, 10, 5);
    (((data << 10) | rem) as u16) ^ MICRO_FORMAT_MASK
}

/// Encodes the 15-bit micro format information for a version, error-correction
/// selection and micro data mask reference (`0..=3`).
///
/// Returns `None` if the version does not offer that error-correction selection,
/// or if `mask` is outside `0..=3`.
pub fn micro_format_info_encode(version: MicroVersion, ec: MicroEcLevel, mask: u8) -> Option<u16> {
    if mask > 3 {
        return None;
    }
    let info = micro_symbol_info(version, ec)?;
    Some(micro_format_info_encode_raw(info.symbol_number, mask))
}

/// Decodes 15 bits of micro format information, correcting up to 3 bit errors.
///
/// Returns the recovered `(symbol info, micro mask reference)`, or `None` if no
/// candidate lies within Hamming distance 3. The `(15, 5)` BCH code has minimum
/// distance 7, so 3 is the exact correction limit (ISO Annex C.3).
pub fn micro_format_info_decode(raw: u16) -> Option<(MicroSymbolInfo, u8)> {
    let raw = raw & 0x7FFF;
    let mut best: Option<(MicroSymbolInfo, u8)> = None;
    let mut best_dist = 4u32;
    for info in MICRO_SYMBOLS.iter() {
        for mask in 0..4u8 {
            let cand = micro_format_info_encode_raw(info.symbol_number, mask);
            let dist = (cand ^ raw).count_ones();
            if dist < best_dist {
                best_dist = dist;
                best = Some((*info, mask));
            }
        }
    }
    best
}

/// Reads the 15-bit micro format information out of a symbol matrix.
///
/// Bit `n` is taken from `MICRO_FORMAT_INFO_POSITIONS[n]`, with bit 0 the least
/// significant. Modules outside the matrix read as light.
///
/// Warning: a light/garbage read does NOT reliably fail to decode. The
/// BCH(15,5) code corrects up to 3 bit errors, so ~56% of all 15-bit words —
/// including the all-light `0`, which decodes to M4-L mask 1 — land within
/// distance 3 of some valid word. Callers (decoders) must not use format-info
/// decode success as evidence of a valid symbol; downstream Reed-Solomon
/// syndrome checks are the real gate.
pub fn micro_format_info_read(matrix: &BitMatrix) -> u16 {
    let mut value = 0u16;
    for (bit, &(x, y)) in MICRO_FORMAT_INFO_POSITIONS.iter().enumerate() {
        if matrix.get(x, y) {
            value |= 1 << bit;
        }
    }
    value
}

/// Writes 15 bits of micro format information into a symbol matrix at the
/// positions given by [`MICRO_FORMAT_INFO_POSITIONS`], bit 0 first.
pub fn micro_format_info_write(matrix: &mut BitMatrix, format: u16) {
    for (bit, &(x, y)) in MICRO_FORMAT_INFO_POSITIONS.iter().enumerate() {
        matrix.set(x, y, (format >> bit) & 1 == 1);
    }
}

/// Returns `true` when the module at column `x`, row `y` belongs to a function
/// pattern or the format information of a micro symbol of this version, and
/// therefore never carries data.
///
/// This is the micro counterpart of
/// [`is_function_module`](crate::is_function_module) and is the single source of
/// truth for both the encoder and the decoder. Coordinates are `(x, y)` =
/// (column, row) from the top-left; out-of-range coordinates return `false`.
///
/// The reserved regions are (ISO 5.3.3.2, 5.3.4, 5.3.5, 7.9.2):
///
/// - the 8 x 8 block `x < 8 && y < 8`: the single 7 x 7 finder pattern plus its
///   one-module separator along row 7 and column 7;
/// - the whole of row 0 and the whole of column 0: the horizontal and vertical
///   timing patterns, which in micro symbols run edge to edge rather than
///   between finders;
/// - column 8 rows 1..=8 and row 8 columns 1..=8: the format information.
///
/// There are no alignment patterns, no version information and no fixed dark
/// module in a micro symbol.
pub fn micro_is_function_module(version: MicroVersion, x: u32, y: u32) -> bool {
    let size = version.size();
    if x >= size || y >= size {
        return false;
    }

    // Finder pattern (rows/columns 0..=6) plus its separator (row 7, column 7).
    if x < 8 && y < 8 {
        return true;
    }

    // Timing patterns: the full top row and the full left column.
    if x == 0 || y == 0 {
        return true;
    }

    // Format information: down column 8 and left along row 8.
    if (x == 8 && (1..=8).contains(&y)) || (y == 8 && (1..=8).contains(&x)) {
        return true;
    }

    false
}

/// Iterator over the data-module positions of a micro symbol, in codeword bit
/// order. Created by [`micro_data_modules`].
///
/// Yields `(x, y)` = (column, row) pairs; the first pair takes the most
/// significant bit of the first codeword, the last pair the least significant
/// bit of the last error correction codeword.
#[derive(Debug, Clone)]
pub struct MicroDataModules {
    version: MicroVersion,
    size: u32,
    /// Right-hand column of the current two-module-wide column pair; `0` once
    /// the iterator is exhausted, since column 0 is always timing.
    col: u32,
    /// Whether the current column pair is traversed bottom-to-top.
    up: bool,
    /// Index `0..size` of the current row within the column pair.
    row_step: u32,
    /// Whether the next module to consider is the right one of the pair.
    right: bool,
}

impl Iterator for MicroDataModules {
    type Item = (u32, u32);

    fn next(&mut self) -> Option<(u32, u32)> {
        loop {
            if self.col == 0 {
                return None;
            }
            if self.row_step >= self.size {
                // Move to the next pair to the left and reverse direction.
                if self.col < 2 {
                    self.col = 0;
                    return None;
                }
                self.col -= 2;
                self.up = !self.up;
                self.row_step = 0;
                self.right = true;
                continue;
            }

            let y = if self.up {
                self.size - 1 - self.row_step
            } else {
                self.row_step
            };
            let x = if self.right { self.col } else { self.col - 1 };

            if self.right {
                self.right = false;
            } else {
                self.right = true;
                self.row_step += 1;
            }

            if !micro_is_function_module(self.version, x, y) {
                return Some((x, y));
            }
        }
    }
}

/// Returns the data-module placement order for a micro symbol: the single source
/// of truth shared by the micro encoder and decoder.
///
/// Following ISO 7.7.3 (which states that "exactly the same principles apply to
/// a micro QR code symbol"), the interleaved codeword stream is treated as one
/// bit stream and placed, most significant bit first, into two-module-wide
/// columns running alternately upwards and downwards from the right edge to the
/// left. Within each column the right module is taken before the left, and
/// function modules are skipped.
///
/// The micro symbol differs from the QR symbol in two ways, both handled here:
/// the vertical timing pattern sits in column 0 rather than column 6, so there
/// is no mid-symbol column skip, and column 0 is never part of a column pair.
///
/// The iterator yields exactly [`MicroVersion::data_modules`] positions — micro
/// symbols have no remainder bits — and the 4-bit final data codeword of M1 and
/// M3 falls naturally into a 2 x 2 square block, as ISO 7.7.3 requires.
#[inline]
pub fn micro_data_modules(version: MicroVersion) -> MicroDataModules {
    let size = version.size();
    MicroDataModules {
        version,
        size,
        col: size - 1,
        up: true,
        row_step: 0,
        right: true,
    }
}
