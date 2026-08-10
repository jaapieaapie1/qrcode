//! Micro QR (M1-M4) encoding — ISO/IEC 18004:2024.
//!
//! This module is a parallel implementation to the QR path in the crate root:
//! [`encode_micro`] mirrors [`encode`](crate::encode), [`MicroOptions`] mirrors
//! [`Options`](crate::Options) and [`MicroEncodeError`] mirrors
//! [`EncodeError`](crate::EncodeError). No existing type gained a variant or a
//! field.
//!
//! Every symbology table, mask, placement order and format-information value is
//! taken from [`qr_core::micro`]; nothing is duplicated here. What this module
//! adds is the encoder-side data path:
//!
//! 1. mode selection restricted to the modes the version offers (ISO Table 2),
//! 2. smallest-fitting version selection over the versions that offer the
//!    requested error-correction selection (ISO Table 13),
//! 3. bit-stream assembly with the version-specific mode indicator, character
//!    count indicator and terminator (ISO 7.4.10, Tables 2 and 3),
//! 4. bit-stream to codeword conversion including the 4-bit final data codeword
//!    of M1 and M3 and its `0000` pad form (ISO 7.4.11),
//! 5. Reed-Solomon over the single block every micro symbol has (ISO 7.6),
//! 6. placement through [`qr_core::micro::micro_data_modules`],
//! 7. data-mask selection by the micro evaluation score of ISO 7.8.3.2, which
//!    is an edge-darkness measure and shares nothing with the N1-N4 penalty of
//!    7.8.3.1 used for QR symbols,
//! 8. format-information placement through
//!    [`qr_core::micro::micro_format_info_write`].
//!
//! # Coordinates
//!
//! `(x, y)` = (column, row), origin top-left, as everywhere else in the
//! workspace.
//!
//! # Scope
//!
//! Single-segment encoding in numeric, alphanumeric or byte mode, matching the
//! QR path. Kanji mode is listed by ISO for M3 and M4 but is not implemented by
//! this crate (see the crate Known Gaps); a payload is never encoded as Kanji,
//! it falls through to byte mode.

use alloc::vec::Vec;

use qr_core::BitMatrix;
use qr_core::Mode;
use qr_core::micro::{
    MicroEcLevel, MicroMode, MicroSymbolInfo, MicroVersion, micro_data_modules,
    micro_format_info_encode, micro_format_info_write, micro_symbol_info,
};
use qr_core::reed_solomon::rs_encode;

use crate::bits::BitBuffer;
use crate::segment;

/// Errors returned by [`encode_micro`].
///
/// A parallel type to [`EncodeError`](crate::EncodeError): the micro ranges and
/// failure modes differ (mask `0..=3`, four versions, and an error-correction
/// selection that is only valid for some versions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicroEncodeError {
    /// The payload does not fit the requested version, or does not fit any micro
    /// version at the requested error-correction selection.
    DataTooLarge,
    /// No encoding mode available in the requested version can represent the
    /// payload — for example non-numeric data in M1, or byte data in M1 or M2
    /// (ISO Table 2).
    UnsupportedMode,
    /// The requested version does not offer the requested error-correction
    /// selection (ISO Table 13), for example `Q` in anything but M4, or
    /// [`MicroEcLevel::Detection`] in anything but M1.
    UnsupportedEcLevel,
    /// A caller-supplied mask id is outside `0..=3`.
    InvalidMask,
}

impl core::fmt::Display for MicroEncodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let msg = match self {
            MicroEncodeError::DataTooLarge => "payload does not fit the requested micro version",
            MicroEncodeError::UnsupportedMode => {
                "no micro mode available in this version can represent the payload"
            }
            MicroEncodeError::UnsupportedEcLevel => {
                "micro version does not offer this error correction level"
            }
            MicroEncodeError::InvalidMask => "micro mask id out of range (0..=3)",
        };
        f.write_str(msg)
    }
}

impl core::error::Error for MicroEncodeError {}

/// Options controlling how a payload is encoded into a micro symbol.
///
/// Leave the optional fields as `None` to let the encoder choose.
#[derive(Debug, Clone, Copy)]
pub struct MicroOptions {
    /// Error-correction selection. Required.
    ///
    /// [`MicroEcLevel::Detection`] is the M1-only "error detection only"
    /// selection of ISO Table 13, so requesting it pins the symbol to M1.
    pub ec: MicroEcLevel,
    /// Force a specific version; otherwise the smallest fitting version that
    /// offers `ec` is used.
    pub version: Option<MicroVersion>,
    /// Force a specific micro data mask (`0..=3`); otherwise the mask with the
    /// highest ISO 7.8.3.2 evaluation score is chosen.
    pub mask: Option<u8>,
}

impl MicroOptions {
    /// Options that auto-select version and mask for the given error-correction
    /// selection.
    pub fn new(ec: MicroEcLevel) -> Self {
        MicroOptions {
            ec,
            version: None,
            mask: None,
        }
    }
}

/// The QR mode whose payload encoding a micro mode reuses.
///
/// The per-mode bit encoding of the payload itself is identical in both
/// symbologies (ISO 7.4.4 to 7.4.7); only the mode indicator, the character
/// count width and the availability differ, and those come from
/// [`qr_core::micro`].
fn qr_mode(mode: MicroMode) -> Mode {
    match mode {
        MicroMode::Numeric => Mode::Numeric,
        MicroMode::Alphanumeric => Mode::Alphanumeric,
        MicroMode::Byte => Mode::Byte,
        MicroMode::Kanji => Mode::Kanji,
    }
}

/// Selects the most compact mode that is available in `version` and can
/// represent every byte of `data`, or `None` if there is none.
///
/// [`MicroMode::Kanji`] is never selected: [`segment::mode_supports`] rejects
/// [`Mode::Kanji`], so byte mode is chosen for such payloads wherever it is
/// available.
fn select_mode(data: &[u8], version: MicroVersion) -> Option<MicroMode> {
    MicroMode::ALL
        .into_iter()
        .find(|&mode| mode.is_available(version) && segment::mode_supports(qr_mode(mode), data))
}

/// Total data bits (mode indicator + character count + payload) for `len`
/// characters in `mode` at `version`, or `None` if the mode is unavailable.
fn total_bits(mode: MicroMode, len: usize, version: MicroVersion) -> Option<usize> {
    let (_, indicator_bits) = mode.indicator(version)?;
    let count_bits = mode.char_count_bits(version)?;
    Some(indicator_bits as usize + count_bits as usize + segment::payload_bits(qr_mode(mode), len))
}

/// Resolves the symbol and the mode to encode with, validating any caller-forced
/// version.
fn resolve(
    data: &[u8],
    options: &MicroOptions,
) -> Result<(MicroSymbolInfo, MicroMode), MicroEncodeError> {
    if let Some(version) = options.version {
        let info =
            micro_symbol_info(version, options.ec).ok_or(MicroEncodeError::UnsupportedEcLevel)?;
        let mode = select_mode(data, version).ok_or(MicroEncodeError::UnsupportedMode)?;
        let bits =
            total_bits(mode, data.len(), version).ok_or(MicroEncodeError::UnsupportedMode)?;
        return if bits <= info.data_bits as usize {
            Ok((info, mode))
        } else {
            Err(MicroEncodeError::DataTooLarge)
        };
    }

    // Smallest fitting version, in M1..M4 order, over the versions that offer
    // the requested error-correction selection. The mode is re-selected per
    // version because availability widens with version (ISO Table 2).
    let mut mode_available = false;
    for version in MicroVersion::ALL {
        let Some(info) = micro_symbol_info(version, options.ec) else {
            continue;
        };
        let Some(mode) = select_mode(data, version) else {
            continue;
        };
        mode_available = true;
        let Some(bits) = total_bits(mode, data.len(), version) else {
            continue;
        };
        if bits <= info.data_bits as usize {
            return Ok((info, mode));
        }
    }

    if mode_available {
        Err(MicroEncodeError::DataTooLarge)
    } else {
        Err(MicroEncodeError::UnsupportedMode)
    }
}

/// Assembles the padded data-codeword stream for a micro symbol.
///
/// The returned slice has [`MicroSymbolInfo::data_codewords`] entries. For M1 and
/// M3 the final entry is the 4-bit half codeword as a **value** in `0..=15`: its
/// high nibble is zero, because a 4-bit codeword enters Reed-Solomon
/// zero-prefixed, following the ISO Annex N.1 precedent for the other 4-bit
/// codeword the standard defines ("The first 4-bit data codeword is prefixed
/// with 0000 to make its length 8 bits for generating the error correction
/// codewords"). Only those 4 low bits are written into the symbol, and their
/// order in the module stream is unchanged by the prefixing
/// (ISO 7.4.11, 7.7.3).
///
/// Returns `None` only if `mode` is unavailable in the symbol's version, which
/// [`resolve`] has already excluded.
fn build_data_codewords(data: &[u8], mode: MicroMode, info: &MicroSymbolInfo) -> Option<Vec<u8>> {
    let version = info.version;
    let data_bits = info.data_bits as usize;
    let half = info.has_half_final_data_codeword();
    // Bits held by the 8-bit data codewords, i.e. everything but the half
    // codeword. Always a multiple of 8.
    let full_codeword_bits = if half { data_bits - 4 } else { data_bits };

    let mut buf = BitBuffer::new();

    let (indicator, indicator_bits) = mode.indicator(version)?;
    buf.push_bits(indicator as u32, indicator_bits);
    buf.push_bits(data.len() as u32, mode.char_count_bits(version)?);
    segment::write_payload(&mut buf, qr_mode(mode), data);

    // Terminator (ISO 7.4.10): omitted or abbreviated when the remaining
    // capacity is shorter than the version's terminator.
    let terminator = core::cmp::min(
        version.terminator_bits() as usize,
        data_bits.saturating_sub(buf.len()),
    );
    for _ in 0..terminator {
        buf.push_bit(false);
    }

    if buf.len() <= full_codeword_bits {
        // Pad to the codeword boundary with zero bits, then fill the remaining
        // 8-bit codewords with the alternating pad codewords (ISO 7.4.11).
        while !buf.len().is_multiple_of(8) {
            buf.push_bit(false);
        }
        let pad = [0xECu8, 0x11u8];
        let mut i = 0;
        while buf.len() < full_codeword_bits {
            buf.push_bits(pad[i % 2] as u32, 8);
            i += 1;
        }
    }
    // Whatever is left is inside the M1/M3 half codeword: zero bits, which is
    // both the padding-bit rule and the `0000` half pad codeword of ISO 7.4.11.
    while buf.len() < data_bits {
        buf.push_bit(false);
    }

    let mut codewords = buf.into_bytes();
    if half && let Some(last) = codewords.last_mut() {
        // MSB-first packing leaves the half codeword's 4 bits in the high nibble
        // of the final byte. Shift it down so the codeword is the value the
        // Reed-Solomon step expects, zero-prefixed rather than zero-suffixed.
        *last >>= 4;
    }
    Some(codewords)
}

/// Packs the module bit stream: every data codeword followed by every error
/// correction codeword, MSB first, with the M1/M3 final data codeword
/// contributing only 4 bits — its low four, since it is held as a value in
/// `0..=15`. The zero prefix it carries for the Reed-Solomon step is not written
/// into the symbol, so the module order is exactly the four bits of the
/// codeword, most significant first.
///
/// The result is `MicroVersion::data_modules` bits long, packed into bytes.
fn build_module_stream(data_codewords: &[u8], ec_codewords: &[u8], half: bool) -> Vec<u8> {
    let mut buf = BitBuffer::new();
    let last = data_codewords.len().saturating_sub(1);
    for (i, &codeword) in data_codewords.iter().enumerate() {
        if half && i == last {
            buf.push_bits(codeword as u32, 4);
        } else {
            buf.push_bits(codeword as u32, 8);
        }
    }
    for &codeword in ec_codewords {
        buf.push_bits(codeword as u32, 8);
    }
    buf.into_bytes()
}

/// Draws the micro function patterns into a blank matrix: the single 7 x 7
/// finder at the top-left with its one-module separator, and the two timing
/// patterns running along row 0 and column 0 to the symbol edge
/// (ISO 5.3.3.2, 5.3.4, 5.3.5).
///
/// The format-information modules are left light; they are written last by
/// [`qr_core::micro::micro_format_info_write`].
fn place_function_patterns(matrix: &mut BitMatrix, version: MicroVersion) {
    let size = version.size();

    for y in 0..7u32 {
        for x in 0..7u32 {
            let outer_ring = x == 0 || x == 6 || y == 0 || y == 6;
            let core = (2..=4).contains(&x) && (2..=4).contains(&y);
            matrix.set(x, y, outer_ring || core);
        }
    }
    // The separator, row 7 and column 7, is light and the matrix starts light.

    // Timing patterns run from the separator to the far edge, alternating and
    // starting dark at even coordinates.
    for i in 8..size {
        matrix.set(i, 0, i.is_multiple_of(2));
        matrix.set(0, i, i.is_multiple_of(2));
    }
}

/// Places the module bit stream into the data modules, in the order given by the
/// shared core iterator.
fn place_data(matrix: &mut BitMatrix, version: MicroVersion, stream: &[u8]) {
    for (n, (x, y)) in micro_data_modules(version).enumerate() {
        let byte = stream[n / 8];
        matrix.set(x, y, (byte >> (7 - (n % 8))) & 1 == 1);
    }
}

/// XORs the micro data mask over the data modules only.
fn apply_mask(matrix: &mut BitMatrix, version: MicroVersion, mask: u8) {
    for (x, y) in micro_data_modules(version) {
        if qr_core::micro::micro_mask_bit(mask, x, y) {
            matrix.toggle(x, y);
        }
    }
}

/// The ISO 7.8.3.2 micro evaluation score of a masked symbol.
///
/// `S1` is the number of dark modules down the right-hand edge and `S2` the
/// number along the lower edge, each excluding the final module of the timing
/// pattern that shares the edge — that is, `(size - 1, 0)` for the right edge
/// and `(0, size - 1)` for the lower edge. The score is
/// `min(S1, S2) * 16 + max(S1, S2)`, and the **highest** score wins, the
/// opposite direction to the QR penalty score, because dark edge modules are
/// what separate the encoding region from the quiet zone.
///
/// The format-information modules never lie on either edge (they occupy column 8
/// and row 8, and the smallest symbol is 11 modules across), so the score is the
/// same whether or not the format information has been written.
fn evaluation_score(matrix: &BitMatrix, version: MicroVersion) -> u32 {
    let size = version.size();
    let mut s1 = 0u32;
    let mut s2 = 0u32;
    for i in 1..size {
        if matrix.get(size - 1, i) {
            s1 += 1;
        }
        if matrix.get(i, size - 1) {
            s2 += 1;
        }
    }
    if s1 <= s2 { s1 * 16 + s2 } else { s2 * 16 + s1 }
}

/// Returns the mask (`0..=3`) with the highest [`evaluation_score`], the lowest
/// id winning a tie.
fn choose_mask(unmasked: &BitMatrix, version: MicroVersion) -> u8 {
    let mut best_mask = 0u8;
    let mut best_score = None;
    for mask in 0..4u8 {
        let mut candidate = unmasked.clone();
        apply_mask(&mut candidate, version, mask);
        let score = evaluation_score(&candidate, version);
        if best_score.is_none_or(|best| score > best) {
            best_score = Some(score);
            best_mask = mask;
        }
    }
    best_mask
}

/// Encodes `data` into a finished Micro QR symbol matrix.
///
/// The returned [`BitMatrix`] contains the finder, separator and timing
/// patterns, the masked data and the format information, ready to be rendered.
/// Micro symbols have no alignment patterns, no version information and no fixed
/// dark module.
///
/// # Errors
///
/// Returns [`MicroEncodeError`] when the payload does not fit, when no mode
/// available in the chosen version can represent it, when the version does not
/// offer the requested error-correction selection, or when a caller-forced mask
/// is out of range.
pub fn encode_micro(data: &[u8], options: MicroOptions) -> Result<BitMatrix, MicroEncodeError> {
    if let Some(mask) = options.mask
        && mask > 3
    {
        return Err(MicroEncodeError::InvalidMask);
    }

    let (info, mode) = resolve(data, &options)?;
    let data_codewords =
        build_data_codewords(data, mode, &info).ok_or(MicroEncodeError::UnsupportedMode)?;
    let ec_codewords = rs_encode(&data_codewords, info.ec_codewords as usize);
    let stream = build_module_stream(
        &data_codewords,
        &ec_codewords,
        info.has_half_final_data_codeword(),
    );

    let version = info.version;
    let mut matrix = BitMatrix::new(version.size());
    place_function_patterns(&mut matrix, version);
    place_data(&mut matrix, version, &stream);

    let mask = match options.mask {
        Some(mask) => mask,
        None => choose_mask(&matrix, version),
    };
    apply_mask(&mut matrix, version, mask);

    let format =
        micro_format_info_encode(version, info.ec, mask).ok_or(MicroEncodeError::InvalidMask)?;
    micro_format_info_write(&mut matrix, format);

    Ok(matrix)
}
