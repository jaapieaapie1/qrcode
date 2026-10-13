//! `qr-encode` — turn a byte payload into a finished QR-symbol [`BitMatrix`].
//!
//! The encoder performs the full ISO/IEC 18004 data path: optimal mixed-mode
//! segmentation, bit-stream assembly (mode indicators, character counts,
//! payloads, terminator and pad codewords), smallest-fitting version selection,
//! Reed-Solomon block splitting and interleaving (via [`qr_core`]), zigzag
//! module placement, evaluation of all eight data masks against the ISO penalty
//! rules and placement of the format (and, for versions >= 7, version)
//! information.
//!
//! # Coordinates
//!
//! Output matrices use `(x, y)` = (column, row) with the origin at the
//! top-left, matching [`qr_core::BitMatrix`].
//!
//! # Segmentation
//!
//! [`encode`] splits the payload into the minimal-bit sequence of numeric,
//! alphanumeric and byte segments by dynamic programming (see the `segment`
//! module); it is never longer than encoding the whole payload in a single
//! mode. Setting [`Options::mode`] overrides this with one forced segment;
//! [`Options::single_segment`] keeps the mode automatic but encodes the whole
//! payload as one segment of the narrowest mode that holds every byte.
//!
//! # EC level boost
//!
//! [`Options::boost_ec`] keeps the version chosen for [`Options::ec`] but raises
//! the level as far as that version still holds the bit stream, turning pad
//! codewords into error correction at no cost in symbol size.
//!
//! # Kanji and Shift-JIS
//!
//! Kanji mode (ISO §7.4.6) transports *Shift-JIS character values*, not bytes: a
//! conformant reader converts the 13-bit values back to Shift-JIS characters.
//! Since this crate performs no character-set conversion and a `&[u8]` carries
//! no encoding, Kanji is **opt-in**:
//!
//! - [`Options::assume_shift_jis`] (default `false`) — the caller asserts the
//!   payload is Shift-JIS, and the segmenter may use Kanji segments where they
//!   are cheaper.
//! - [`Options::mode`] = [`Mode::Kanji`] — one forced Kanji segment.
//!
//! By default UTF-8 (and any other non-Shift-JIS) input goes to byte mode
//! unchanged, even where its bytes happen to fall inside the Shift-JIS ranges —
//! which ordinary UTF-8 CJK text does constantly. Setting the flag on a payload
//! that is *not* Shift-JIS produces mojibake on every reader other than this
//! workspace's own byte-transparent decoder.
//!
//! # Extended Channel Interpretation
//!
//! [`Options::eci`] emits an ECI header (ISO §7.4.3.2) ahead of the data
//! segments — for example ECI 26 to declare a UTF-8 byte payload. Micro QR has
//! no ECI mode and `MicroOptions` accordingly has no such field.
//!
//! # Features
//!
//! `micro` (default) adds the Micro QR encoder in the `micro` module:
//! `encode_micro`, `MicroOptions` and `MicroEncodeError`, parallel to
//! [`encode`], [`Options`] and [`EncodeError`]. Opt out with
//! `--no-default-features`; the QR path is unaffected either way. The names are
//! written unlinked here because they do not exist when the feature is off.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

#[cfg(test)]
extern crate std;

mod bits;
mod matrix;
mod packed;
mod segment;

#[cfg(feature = "micro")]
pub mod micro;

#[cfg(feature = "micro")]
pub use micro::{MicroEncodeError, MicroOptions, encode_micro};

use alloc::vec;
use alloc::vec::Vec;
use bits::BitBuffer;
use qr_core::{BitMatrix, EcLevel, Mode, Version, ec_blocks};
use segment::Segment;

/// Errors returned by [`encode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeError {
    /// The payload does not fit in the requested (or any) version at the given
    /// EC level.
    DataTooLarge,
    /// A caller-supplied [`Mode`] cannot represent the payload, or is a mode
    /// that carries no data ([`Mode::Eci`] — use [`Options::eci`] instead).
    ModeMismatch,
    /// A caller-supplied version is out of the `1..=40` range.
    InvalidVersion,
    /// A caller-supplied mask id is outside `0..=7`.
    InvalidMask,
    /// A caller-supplied ECI assignment number is outside the six-digit range
    /// `0..=999_999` (ISO/IEC 18004 Table 4).
    InvalidEci,
}

impl core::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let msg = match self {
            EncodeError::DataTooLarge => "payload does not fit the requested version",
            EncodeError::ModeMismatch => "mode cannot represent the payload",
            EncodeError::InvalidVersion => "version out of range (1..=40)",
            EncodeError::InvalidMask => "mask id out of range (0..=7)",
            EncodeError::InvalidEci => "ECI assignment number out of range (0..=999999)",
        };
        f.write_str(msg)
    }
}

impl core::error::Error for EncodeError {}

/// Options controlling how a payload is encoded.
///
/// Leave optional fields as `None` to let the encoder choose automatically.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Error-correction level. Required.
    pub ec: EcLevel,
    /// Force a specific version; otherwise the smallest fitting version is used.
    pub version: Option<Version>,
    /// Force a specific data mask (`0..=7`); otherwise the lowest-penalty mask
    /// is chosen.
    pub mask: Option<u8>,
    /// Force the whole payload into one segment of this mode; otherwise the
    /// minimal-bit mixed-mode segmentation is computed from the payload.
    ///
    /// [`Mode::Eci`] is rejected — set [`Options::eci`] instead.
    pub mode: Option<Mode>,
    /// Emit an ECI header with this six-digit assignment number ahead of the
    /// data segments (ISO/IEC 18004 §7.4.3.2); `None` leaves the default
    /// interpretation in force. ECI 26 declares a UTF-8 byte payload.
    pub eci: Option<u32>,
    /// Assert that the payload is **Shift-JIS**, allowing the segmenter to use
    /// 13-bit Kanji segments (ISO §7.4.6) where they are cheaper than byte mode.
    ///
    /// Defaults to `false`, which keeps every byte pair in byte mode. Set it
    /// only when the bytes really are Shift-JIS: Kanji mode carries Shift-JIS
    /// *character values*, so a conformant reader will render non-Shift-JIS
    /// bytes placed in a Kanji segment as mojibake. Byte values alone cannot
    /// distinguish the two — UTF-8 CJK text lands inside the Shift-JIS ranges
    /// routinely — which is why this is a caller assertion and not inferred.
    ///
    /// Ignored when [`Options::mode`] forces a mode; forcing
    /// [`Mode::Kanji`] is the other way to ask for Kanji segments.
    pub assume_shift_jis: bool,
    /// Encode the whole payload as **one** segment of the narrowest mode that
    /// holds every byte — numeric, alphanumeric, Kanji (only with
    /// [`Options::assume_shift_jis`]), then byte — instead of the minimal-bit
    /// mixed-mode segmentation. Some generators (printer firmware among them)
    /// encode this way; matching them module for module needs it.
    ///
    /// Defaults to `false`. Ignored when [`Options::mode`] forces a mode.
    pub single_segment: bool,
    /// After the version is chosen (or forced) for [`Options::ec`], raise the
    /// error-correction level to the highest one at which that same version
    /// still holds the data. Never lowers the level or grows the symbol.
    ///
    /// Defaults to `false`, which encodes at exactly [`Options::ec`].
    pub boost_ec: bool,
}

impl Options {
    /// Options that auto-select version, mask and segmentation for the given EC
    /// level, with no ECI header and Kanji mode off.
    pub fn new(ec: EcLevel) -> Self {
        Options {
            ec,
            version: None,
            mask: None,
            mode: None,
            eci: None,
            assume_shift_jis: false,
            single_segment: false,
            boost_ec: false,
        }
    }
}

/// Data-codeword capacity (in bits) of `version` at `ec`.
fn data_capacity_bits(version: Version, ec: EcLevel) -> usize {
    ec_blocks(version, ec).total_data_codewords() as usize * 8
}

/// Inclusive version bounds of the three ISO Table 3 character-count groups.
/// Segmentation depends on the version only through this grouping, so the
/// dynamic program runs once per group rather than once per version.
const VERSION_GROUPS: [(u8, u8); 3] = [(1, 9), (10, 26), (27, 40)];

/// Splits `data` into segments for `version`: one forced segment when the
/// caller pinned a mode, one segment of the narrowest mode when the caller
/// asked for a single segment, otherwise the minimal-bit segmentation.
///
/// Kanji segments are only ever produced from an explicit caller assertion —
/// `Options::mode = Some(Mode::Kanji)` here, or `Options::assume_shift_jis`
/// reaching the dynamic program.
fn segments_for(
    data: &[u8],
    version: Version,
    options: &Options,
) -> Result<Vec<Segment>, EncodeError> {
    match options.mode {
        Some(m) => {
            if segment::mode_supports(m, data) {
                Ok(vec![Segment::new(m, 0, data.len())])
            } else {
                Err(EncodeError::ModeMismatch)
            }
        }
        None if options.single_segment => Ok(vec![Segment::new(
            segment::narrowest_mode(data, options.assume_shift_jis),
            0,
            data.len(),
        )]),
        None => Ok(segment::optimal_segments(
            data,
            version,
            options.assume_shift_jis,
        )),
    }
}

/// Chooses the version and the segmentation to encode with.
///
/// With no forced version this walks the character-count groups in order,
/// segmenting once per group and returning the first version whose data
/// capacity holds the resulting bit stream — still the smallest fitting
/// version, because the bit count is constant inside a group.
///
/// Also returns the stream's length in bits, for [`boosted_ec`].
fn resolve_plan(
    data: &[u8],
    options: &Options,
) -> Result<(Version, Vec<Segment>, usize), EncodeError> {
    let eci_bits = match options.eci {
        Some(v) => segment::eci_bits(v).ok_or(EncodeError::InvalidEci)?,
        None => 0,
    };

    if let Some(v) = options.version {
        let segments = segments_for(data, v, options)?;
        let bits =
            segment::segments_bits(&segments, v).ok_or(EncodeError::ModeMismatch)? + eci_bits;
        return if bits <= data_capacity_bits(v, options.ec) {
            Ok((v, segments, bits))
        } else {
            Err(EncodeError::DataTooLarge)
        };
    }

    for (first, last) in VERSION_GROUPS {
        let anchor = Version::new(first).map_err(|_| EncodeError::InvalidVersion)?;
        let segments = segments_for(data, anchor, options)?;
        let bits =
            segment::segments_bits(&segments, anchor).ok_or(EncodeError::ModeMismatch)? + eci_bits;
        for n in first..=last {
            let v = Version::new(n).map_err(|_| EncodeError::InvalidVersion)?;
            if bits <= data_capacity_bits(v, options.ec) {
                return Ok((v, segments, bits));
            }
        }
    }
    Err(EncodeError::DataTooLarge)
}

/// The highest EC level, from `ec` up, whose capacity in `version` holds
/// `bits`. The caller guarantees `bits` fits at `ec` itself.
fn boosted_ec(version: Version, ec: EcLevel, bits: usize) -> EcLevel {
    [EcLevel::H, EcLevel::Q, EcLevel::M]
        .into_iter()
        .take_while(|&higher| higher != ec)
        .find(|&higher| bits <= data_capacity_bits(version, higher))
        .unwrap_or(ec)
}

/// Assembles the padded data-codeword stream (before block splitting).
fn build_data_codewords(
    data: &[u8],
    segments: &[Segment],
    eci: Option<u32>,
    version: Version,
    ec: EcLevel,
) -> Vec<u8> {
    let capacity_bits = data_capacity_bits(version, ec);
    let mut buf = BitBuffer::new();

    if let Some(value) = eci {
        segment::write_eci(&mut buf, value);
    }
    segment::write_segments(&mut buf, data, segments, version);

    // Terminator: up to four zero bits, not exceeding capacity.
    let terminator = core::cmp::min(4, capacity_bits.saturating_sub(buf.len()));
    for _ in 0..terminator {
        buf.push_bit(false);
    }
    // Pad to a byte boundary.
    while !buf.len().is_multiple_of(8) {
        buf.push_bit(false);
    }

    let mut codewords = buf.into_bytes();
    let capacity_bytes = capacity_bits / 8;
    let pad = [0xECu8, 0x11u8];
    let mut i = 0;
    while codewords.len() < capacity_bytes {
        codewords.push(pad[i % 2]);
        i += 1;
    }
    codewords
}

/// Encodes `data` into a finished QR-symbol matrix.
///
/// The returned [`BitMatrix`] contains the function patterns, masked data,
/// format information and (for versions >= 7) version information, ready to be
/// rendered.
///
/// # Errors
///
/// Returns [`EncodeError`] when the payload does not fit, or when a
/// caller-forced mode, version, mask or ECI number is invalid.
pub fn encode(data: &[u8], options: Options) -> Result<BitMatrix, EncodeError> {
    if let Some(m) = options.mask
        && m > 7
    {
        return Err(EncodeError::InvalidMask);
    }

    let (version, segments, bits) = resolve_plan(data, &options)?;
    let ec = if options.boost_ec {
        boosted_ec(version, options.ec, bits)
    } else {
        options.ec
    };

    let data_codewords = build_data_codewords(data, &segments, options.eci, version, ec);
    let blocks = ec_blocks(version, ec);
    let interleaved = matrix::interleave(&data_codewords, blocks);

    // Placement, masking and (when no mask is forced) the eight-candidate
    // penalty comparison all run on the word-packed grid.
    Ok(packed::build_symbol(
        version,
        ec,
        &interleaved,
        options.mask,
    ))
}

#[cfg(test)]
mod tests;
