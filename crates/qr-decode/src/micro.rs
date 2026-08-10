//! Micro QR (versions M1-M4) decoding, per ISO/IEC 18004:2024.
//!
//! This module is gated behind the `micro` cargo feature (on by default) and is
//! deliberately *parallel* to the full-QR decoder: [`decode_micro`] sits beside
//! [`decode`](crate::decode) and [`DecodedMicro`] beside
//! [`Decoded`](crate::Decoded). Nothing here changes the meaning of an existing
//! type, and no existing signature moves.
//!
//! Every table, placement rule and mask predicate comes from
//! [`qr_core::micro`]; this module owns only the decode-side pipeline:
//!
//! 1. side length `11 / 13 / 15 / 17` -> [`MicroVersion`];
//! 2. read the single 15-bit format information strip and BCH-correct it, which
//!    yields the ISO Table 13 symbol number (version *and* error-correction
//!    selection) plus the data mask;
//! 3. cross-check that symbol number against the version implied by the size —
//!    micro symbols carry only *one* format information copy, so there is no
//!    second copy to vote against a bad read;
//! 4. walk [`micro_data_modules`], un-masking as it goes, into the single
//!    Reed-Solomon block;
//! 5. correct (or, for M1, merely verify) that block;
//! 6. parse the micro bit stream into the payload.
//!
//! # Why the Reed-Solomon step is the validity gate
//!
//! [`micro_format_info_read`] is documented as *not* a rejection test: the
//! `(15, 5)` BCH code corrects three bit errors, so roughly 56 % of arbitrary
//! 15-bit words — including the all-light word — land within distance 3 of a
//! valid one. Step 3 removes six of the eight surviving symbol numbers for a
//! given size, and step 5 removes essentially everything else: a random
//! codeword block passes the syndrome check with probability around
//! `256^-(d - r)`. Garbage therefore surfaces as
//! [`DecodeError::BlockCorrection`], not as a bogus payload.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use qr_core::BitMatrix;
use qr_core::micro::{
    MicroEcLevel, MicroMode, MicroSymbolInfo, MicroVersion, micro_data_modules,
    micro_format_info_decode, micro_format_info_read, micro_mask_bit,
};
use qr_core::reed_solomon::{rs_correct, rs_encode};

use crate::bits::BitReader;
use crate::segment::{push_alphanumeric, push_byte, push_kanji, push_numeric};
use crate::{DecodeError, transpose};

/// The result of decoding a Micro QR symbol.
///
/// The parallel of [`Decoded`](crate::Decoded), with the micro version and
/// error-correction types in place of their full-QR counterparts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedMicro {
    /// The raw payload bytes (ASCII for numeric/alphanumeric, verbatim for
    /// byte mode, Shift-JIS for Kanji).
    pub bytes: Vec<u8>,
    /// The payload decoded as UTF-8, if it is valid UTF-8. Micro symbols have
    /// no ECI, so this is plain best-effort interpretation of the bytes.
    pub text: Option<String>,
    /// The symbol version, M1 through M4.
    pub version: MicroVersion,
    /// The error-correction selection read from the format information. M1
    /// always reports [`MicroEcLevel::Detection`].
    pub ec: MicroEcLevel,
    /// The micro data-mask pattern (`0..=3`) read from the format information.
    pub mask: u8,
    /// Number of codewords Reed-Solomon had to correct. Always `0` for M1,
    /// whose error-correction codewords only detect.
    pub errors_corrected: usize,
}

/// Decodes a Micro QR symbol from its module grid.
///
/// `matrix` must be a square grid whose side length is `11`, `13`, `15` or `17`
/// (versions M1-M4); a set module (`true`) is dark. If the straight decode
/// fails the matrix is transposed and retried, transparently handling mirrored
/// symbols exactly as [`decode`](crate::decode) does.
///
/// # Errors
///
/// The shared [`DecodeError`] is reused unchanged; for micro symbols the
/// variants mean:
///
/// - [`InvalidSize`](DecodeError::InvalidSize) — the side length is not
///   `11 / 13 / 15 / 17`.
/// - [`FormatInfo`](DecodeError::FormatInfo) — the 15-bit format information is
///   further than three bit errors from every valid word.
/// - [`VersionInfo`](DecodeError::VersionInfo) — the format information decoded,
///   but its ISO Table 13 symbol number names a version that contradicts the
///   symbol size.
/// - [`BlockCorrection`](DecodeError::BlockCorrection) — the Reed-Solomon block
///   carried more errors than the version and level may correct. For M1 this
///   means the error *detection* codewords did not check out.
/// - [`Malformed`](DecodeError::Malformed) — the corrected data did not parse as
///   a valid micro bit stream.
pub fn decode_micro(matrix: &BitMatrix) -> Result<DecodedMicro, DecodeError> {
    match decode_micro_oriented(matrix) {
        Ok(decoded) => Ok(decoded),
        Err(first) => {
            let mirrored = transpose(matrix);
            decode_micro_oriented(&mirrored).map_err(|_| first)
        }
    }
}

/// Decodes without attempting the mirror retry.
fn decode_micro_oriented(matrix: &BitMatrix) -> Result<DecodedMicro, DecodeError> {
    let version = MicroVersion::from_size(matrix.size()).ok_or(DecodeError::InvalidSize)?;

    let raw = micro_format_info_read(matrix);
    let (info, mask) = micro_format_info_decode(raw).ok_or(DecodeError::FormatInfo)?;

    // A micro symbol carries one format information copy only, and the BCH code
    // happily "corrects" most random words, so the symbol number has to be
    // reconciled with the geometry before anything downstream trusts it.
    if info.version != version {
        return Err(DecodeError::VersionInfo);
    }

    let block = read_block(matrix, &info, mask);
    let (codewords, errors_corrected) = correct_block(block, &info)?;
    let stream = data_bit_stream(&codewords, &info);
    let bytes = parse_micro_segments(&stream, &info).ok_or(DecodeError::Malformed)?;
    let text = String::from_utf8(bytes.clone()).ok();

    Ok(DecodedMicro {
        bytes,
        text,
        version,
        ec: info.ec,
        mask,
        errors_corrected,
    })
}

/// Bit width of codeword `index` within the symbol's single block: 4 for the
/// final data codeword of M1 and M3, 8 everywhere else (ISO 7.4.11).
fn codeword_bits(info: &MicroSymbolInfo, index: usize) -> u32 {
    if info.has_half_final_data_codeword() && index + 1 == info.data_codewords as usize {
        4
    } else {
        8
    }
}

/// Walks the shared placement iterator, un-masking each module, and packs the
/// result into the symbol's single Reed-Solomon block.
///
/// The 4-bit final data codeword of M1 and M3 is zero-extended into the low
/// nibble of its byte, matching the "prefixed with 0000" convention ISO states
/// for the analogous short codeword in Annex N; every other codeword is a
/// plain byte.
fn read_block(m: &BitMatrix, info: &MicroSymbolInfo, mask: u8) -> Vec<u8> {
    let total = info.total_codewords as usize;
    let mut block = vec![0u8; total];

    let mut index = 0usize;
    let mut bit = 0u32;
    for (x, y) in micro_data_modules(info.version) {
        if index >= total {
            break;
        }
        let width = codeword_bits(info, index);
        if m.get(x, y) ^ micro_mask_bit(mask, x, y) {
            block[index] |= 1 << (width - 1 - bit);
        }
        bit += 1;
        if bit == width {
            index += 1;
            bit = 0;
        }
    }
    block
}

/// Corrects the block and returns `(data codewords, corrected codeword count)`.
///
/// Two regimes, per ISO 7.5.1:
///
/// - `error_correction_capacity == 0` (version M1): the two codewords are error
///   *detection* only. The syndromes are checked by recomputing them — which for
///   a systematic code is exactly "are all syndromes zero?" — and a non-zero
///   result is rejected outright. No correction is ever attempted, because a
///   one-error correction here would be a misdecode, not a recovery.
/// - otherwise: Reed-Solomon correction, capped at the version's published
///   correction capacity `r`. The remaining `p` codewords are misdecode
///   protection (ISO Formula 3, `e + 2t <= d - p`) and must detect rather than
///   correct, so a block that needed more than `r` corrections is rejected even
///   though the generic `d / 2` bound would have allowed it.
fn correct_block(
    mut block: Vec<u8>,
    info: &MicroSymbolInfo,
) -> Result<(Vec<u8>, usize), DecodeError> {
    let data_len = info.data_codewords as usize;
    let ec_len = info.ec_codewords as usize;

    let corrected = if info.error_correction_capacity == 0 {
        if rs_encode(&block[..data_len], ec_len).as_slice() != &block[data_len..] {
            return Err(DecodeError::BlockCorrection);
        }
        0
    } else {
        let fixed = rs_correct(&mut block, ec_len).map_err(|_| DecodeError::BlockCorrection)?;
        if fixed > info.error_correction_capacity as usize {
            return Err(DecodeError::BlockCorrection);
        }
        fixed
    };

    // The half codeword is only ever four bits wide, so a corrected value with
    // anything in its high nibble is a mis-correction rather than a recovery.
    if info.has_half_final_data_codeword() && block[data_len - 1] & 0xF0 != 0 {
        return Err(DecodeError::BlockCorrection);
    }

    block.truncate(data_len);
    Ok((block, corrected))
}

/// Repacks the corrected data codewords into a contiguous MSB-first bit stream.
///
/// For M1 and M3 the final codeword holds its four bits in the low nibble; the
/// bit stream needs them at the top of the trailing byte so that a plain
/// MSB-first reader walks `data_bits` bits without a gap.
fn data_bit_stream(codewords: &[u8], info: &MicroSymbolInfo) -> Vec<u8> {
    let mut stream = codewords.to_vec();
    if info.has_half_final_data_codeword()
        && let Some(last) = stream.last_mut()
    {
        *last <<= 4;
    }
    stream
}

/// Parses the micro data bit stream into its payload bytes.
///
/// A micro segment is a version-sized mode indicator (0, 1, 2 or 3 bits for
/// M1-M4; ISO Table 2) followed by a mode- and version-sized character count
/// (ISO Table 3) and the data itself. There is no ECI, FNC1 or structured
/// append in a micro symbol (ISO 7.4.3, 7.4.9).
///
/// The terminator (ISO 7.4.10) is `2n + 1` zero bits for version Mn, which is
/// exactly a numeric mode indicator followed by a zero character count — so the
/// loop stops on that pattern, and stops early when fewer bits remain than a
/// header needs (the abbreviated terminator case). Pad codewords are therefore
/// never parsed as segments.
fn parse_micro_segments(stream: &[u8], info: &MicroSymbolInfo) -> Option<Vec<u8>> {
    let version = info.version;
    let indicator_bits = version.mode_indicator_bits() as u32;
    let mut reader = BitReader::with_bit_len(stream, info.data_bits as usize);
    let mut out = Vec::new();

    loop {
        if reader.remaining() < indicator_bits as usize {
            break;
        }
        let indicator = reader.read(indicator_bits)?;
        let mode = *MicroMode::ALL.get(indicator as usize)?;
        if !mode.is_available(version) {
            return None;
        }

        let count_bits = mode.char_count_bits(version)? as u32;
        if reader.remaining() < count_bits as usize {
            break;
        }
        let count = reader.read(count_bits)? as usize;
        if mode == MicroMode::Numeric && count == 0 {
            break;
        }

        match mode {
            MicroMode::Numeric => push_numeric(&mut reader, count, &mut out)?,
            MicroMode::Alphanumeric => push_alphanumeric(&mut reader, count, &mut out)?,
            MicroMode::Byte => push_byte(&mut reader, count, &mut out)?,
            MicroMode::Kanji => push_kanji(&mut reader, count, &mut out)?,
        }
    }

    Some(out)
}
