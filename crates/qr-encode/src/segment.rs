//! Data analysis, optimal mode segmentation and per-mode bit encoding.
//!
//! The encoder splits the payload into a sequence of [`Segment`]s — one 4-bit
//! mode indicator, character-count indicator and payload block each (ISO/IEC
//! 18004 §7.4.1) — chosen by a dynamic program that minimises the total bit
//! count over the data modes: numeric, alphanumeric and byte always, plus kanji
//! when the caller opts in (see below).
//!
//! # Shift-JIS contract
//!
//! Kanji mode (§7.4.6) does not carry bytes — it carries *Shift-JIS character
//! values*. A conformant reader converts the 13-bit values back to Shift-JIS
//! characters, so putting non-Shift-JIS bytes in a Kanji segment produces
//! mojibake on every scanner that is not this workspace's own (deliberately
//! byte-transparent) decoder.
//!
//! This crate performs **no** character-set conversion and cannot tell from a
//! `&[u8]` what encoding the caller holds — ordinary UTF-8 text lands inside the
//! Shift-JIS ranges constantly. Kanji is therefore **opt-in**: the dynamic
//! program only considers Kanji edges when the caller asserts the payload is
//! Shift-JIS via `Options::assume_shift_jis` (or forces `Mode::Kanji`
//! outright). With the default off, a byte pair that happens to look like
//! Shift-JIS stays in byte mode.

use crate::bits::BitBuffer;
use alloc::vec;
use alloc::vec::Vec;
use qr_core::{Mode, Version};

/// Maps an ASCII byte to its alphanumeric value (ISO Table 5), or `None` if the
/// byte is not in the 45-character alphanumeric set.
fn alnum_value(b: u8) -> Option<u32> {
    let v = match b {
        b'0'..=b'9' => b - b'0',
        b'A'..=b'Z' => b - b'A' + 10,
        b' ' => 36,
        b'$' => 37,
        b'%' => 38,
        b'*' => 39,
        b'+' => 40,
        b'-' => 41,
        b'.' => 42,
        b'/' => 43,
        b':' => 44,
        _ => return None,
    };
    Some(v as u32)
}

fn is_numeric(b: u8) -> bool {
    b.is_ascii_digit()
}

fn is_alnum(b: u8) -> bool {
    alnum_value(b).is_some()
}

/// Converts a Shift-JIS double-byte character to its 13-bit Kanji-mode value
/// (ISO/IEC 18004 §7.4.6), or `None` if `(b1, b2)` is not an encodable pair.
///
/// Per §7.4.6 the 16-bit Shift-JIS code has `8140` subtracted (for codes
/// `8140`–`9FFC`) or `C140` subtracted (for codes `E040`–`EBBF`); the most
/// significant byte of the difference is multiplied by `C0` and the least
/// significant byte added, giving a value in `0..=0x1FFF`.
///
/// The `lo > 0xBF` guard is what makes the mapping injective, so the decoder's
/// inverse reproduces `(b1, b2)` exactly: without it, codes `8200` (difference
/// `00C0`) and `8240` (difference `0100`) would both yield value `0xC0`. It only
/// rejects byte pairs that are not valid Shift-JIS to begin with.
pub fn kanji_value(b1: u8, b2: u8) -> Option<u32> {
    let code = ((b1 as u32) << 8) | b2 as u32;
    let diff = if (0x8140..=0x9FFC).contains(&code) {
        code - 0x8140
    } else if (0xE040..=0xEBBF).contains(&code) {
        code - 0xC140
    } else {
        return None;
    };
    let lo = diff & 0xFF;
    if lo > 0xBF {
        return None;
    }
    // Always <= 0x1FFF, so it always fits 13 bits: the ranges above cap the
    // difference at 0x2A7F (code EBBF), giving 0x2A * 0xC0 + 0x7F = 0x1FFF, and
    // every smaller in-range difference yields at most 0x29 * 0xC0 + 0xBF =
    // 0x1F7F. Do not widen the ranges without re-checking this.
    Some((diff >> 8) * 0xC0 + lo)
}

/// Returns `true` if `data[i..i + 2]` is an encodable Shift-JIS pair.
fn is_kanji_pair(data: &[u8], i: usize) -> bool {
    i + 1 < data.len() && kanji_value(data[i], data[i + 1]).is_some()
}

/// Returns `true` if every byte of `data` can be encoded in `mode`.
///
/// [`Mode::Kanji`] requires an even length whose every byte pair is a valid
/// Shift-JIS double-byte character. [`Mode::Eci`] is a header, not a data mode,
/// and is never supported.
pub fn mode_supports(mode: Mode, data: &[u8]) -> bool {
    match mode {
        Mode::Numeric => data.iter().all(|&b| is_numeric(b)),
        Mode::Alphanumeric => data.iter().all(|&b| is_alnum(b)),
        Mode::Byte => true,
        Mode::Kanji => {
            data.len().is_multiple_of(2)
                && data
                    .chunks_exact(2)
                    .all(|p| kanji_value(p[0], p[1]).is_some())
        }
        Mode::Eci => false,
    }
}

/// The narrowest mode that holds every byte of `data` as a single segment:
/// numeric, alphanumeric, Kanji (only when `allow_kanji`), else byte.
pub fn narrowest_mode(data: &[u8], allow_kanji: bool) -> Mode {
    if mode_supports(Mode::Numeric, data) {
        Mode::Numeric
    } else if mode_supports(Mode::Alphanumeric, data) {
        Mode::Alphanumeric
    } else if allow_kanji && mode_supports(Mode::Kanji, data) {
        Mode::Kanji
    } else {
        Mode::Byte
    }
}

/// Number of payload bits (excluding mode indicator and char-count indicator)
/// needed to encode `len` characters in `mode`.
///
/// For [`Mode::Kanji`] `len` is the number of Kanji *characters*, i.e. half the
/// number of Shift-JIS bytes.
pub fn payload_bits(mode: Mode, len: usize) -> usize {
    match mode {
        Mode::Numeric => {
            let groups = len / 3;
            let rem = len % 3;
            groups * 10
                + match rem {
                    1 => 4,
                    2 => 7,
                    _ => 0,
                }
        }
        Mode::Alphanumeric => (len / 2) * 11 + (len % 2) * 6,
        Mode::Byte => len * 8,
        Mode::Kanji => len * 13,
        Mode::Eci => 0,
    }
}

// ---- segments -------------------------------------------------------------

/// One mode-homogeneous run of the payload, as a byte range into `data`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    /// Mode this run is encoded in.
    pub mode: Mode,
    /// Inclusive start byte offset into the payload.
    pub start: usize,
    /// Exclusive end byte offset into the payload.
    pub end: usize,
}

impl Segment {
    /// Creates a segment covering `data[start..end]` in `mode`.
    pub fn new(mode: Mode, start: usize, end: usize) -> Self {
        Segment { mode, start, end }
    }

    /// The value written into the character-count indicator: bytes for numeric,
    /// alphanumeric and byte mode, byte *pairs* for Kanji mode (ISO §7.4.3.1).
    pub fn char_count(&self) -> usize {
        let len = self.end - self.start;
        match self.mode {
            Mode::Kanji => len / 2,
            _ => len,
        }
    }
}

/// Total bits (mode indicators + char counts + payloads) for `segments` at
/// `version`. `None` if a segment carries a mode with no char-count indicator.
pub fn segments_bits(segments: &[Segment], version: Version) -> Option<usize> {
    let mut total = 0usize;
    for s in segments {
        let cc = s.mode.char_count_bits(version)? as usize;
        total += 4 + cc + payload_bits(s.mode, s.char_count());
    }
    Some(total)
}

/// Bits consumed by an ECI header — the 4-bit ECI mode indicator plus a one-,
/// two- or three-codeword designator (ISO/IEC 18004 §7.4.3.2, Table 4).
///
/// `None` when `value` exceeds the six-digit assignment-number range.
pub fn eci_bits(value: u32) -> Option<usize> {
    match value {
        0..=127 => Some(4 + 8),
        128..=16_383 => Some(4 + 16),
        16_384..=999_999 => Some(4 + 24),
        _ => None,
    }
}

/// Appends the ECI mode indicator and designator for `value` (ISO Table 4).
///
/// The shortest of the three encodings is used, as §7.4.3.2 prefers.
pub fn write_eci(buf: &mut BitBuffer, value: u32) {
    buf.push_bits(Mode::Eci.indicator() as u32, 4);
    if value <= 127 {
        buf.push_bits(value, 8);
    } else if value <= 16_383 {
        buf.push_bits((0b10 << 14) | value, 16);
    } else {
        buf.push_bits((0b110 << 21) | value, 24);
    }
}

// ---- optimal segmentation -------------------------------------------------

/// The four data modes, in dynamic-program state order.
const MODES: [Mode; 4] = [Mode::Numeric, Mode::Alphanumeric, Mode::Byte, Mode::Kanji];
const M_NUMERIC: usize = 0;
const M_ALNUM: usize = 1;
const M_BYTE: usize = 2;
const M_KANJI: usize = 3;
const N_MODES: usize = 4;

/// Unreachable dynamic-program state.
const INF: u32 = u32::MAX;
/// Back-pointer: this state was reached by encoding more characters in the same
/// mode.
const BP_CONTINUE: u8 = 0xFE;
/// Back-pointer: this state is the start of the payload.
const BP_START: u8 = 0xFF;

/// Costs are accumulated in sixths of a bit so the fractional per-character
/// widths (10/3 for numeric, 11/2 for alphanumeric) stay exact; a segment's
/// true bit length is the running total rounded up to a whole bit.
fn ceil_bits(sixths: u32) -> u32 {
    sixths.div_ceil(6) * 6
}

/// Per-character cost, in sixths of a bit, of each mode's payload encoding.
/// Kanji's 13 bits cover a two-byte character.
const STEP: [u32; N_MODES] = [20, 33, 48, 78];

/// Computes the minimal-bit segmentation of `data` for `version`.
///
/// The result depends on `version` only through its ISO Table 3 version group
/// (`1..=9`, `10..=26`, `27..=40`), because that is all the character-count
/// widths — and therefore the per-segment header costs — depend on.
///
/// `allow_kanji` gates the Kanji edges. It must only be set when the caller has
/// asserted the payload is Shift-JIS: Kanji mode transports Shift-JIS character
/// values, not bytes, so inferring it from byte values alone turns UTF-8 text
/// into mojibake on any conformant reader (see the module docs).
///
/// # Algorithm
///
/// A shortest-path dynamic program over states `(byte offset, mode)`. Edges are
/// either "encode the next character in the current mode" (cost [`STEP`], one
/// byte, or two for Kanji) or "close the current segment and open a new one"
/// (cost: round the running total up to a whole bit, then add the new
/// segment's `4 + char-count` header). Because every single-mode encoding is
/// itself a path through this graph, the result is never longer than the best
/// single-mode encoding.
///
/// Empty input yields one empty byte-mode segment, matching a bare `0100`
/// zero-length segment.
pub fn optimal_segments(data: &[u8], version: Version, allow_kanji: bool) -> Vec<Segment> {
    if data.is_empty() {
        return vec![Segment::new(Mode::Byte, 0, 0)];
    }
    let n = data.len();
    let mut head = [0u32; N_MODES];
    for (j, h) in head.iter_mut().enumerate() {
        let bits = MODES[j].char_count_bits(version);
        debug_assert!(
            bits.is_some(),
            "every data mode has a char-count indicator; MODES holds no Mode::Eci"
        );
        // Falling back to the widest indicator can only over-price a segment, so
        // an invariant break degrades the choice rather than the bit stream.
        *h = (4 + bits.unwrap_or(16) as u32) * 6;
    }

    let mut cost = vec![[INF; N_MODES]; n + 1];
    let mut back = vec![[BP_START; N_MODES]; n + 1];
    cost[0] = head;
    if !allow_kanji {
        cost[0][M_KANJI] = INF;
    }

    for i in 0..=n {
        // Mode switches happen at a fixed offset, so relax them from a snapshot:
        // switching twice in a row is never cheaper than switching once.
        let before = cost[i];
        for j in 0..N_MODES {
            if j == M_KANJI && !allow_kanji {
                // No outgoing Kanji edges exist, so opening a Kanji segment
                // could only strand the path (or, at i == n, name an empty
                // segment). Leave the state unreachable.
                continue;
            }
            for (k, &from) in before.iter().enumerate() {
                if k == j || from == INF {
                    continue;
                }
                let candidate = ceil_bits(from) + head[j];
                if candidate < cost[i][j] {
                    cost[i][j] = candidate;
                    back[i][j] = k as u8;
                }
            }
        }
        if i == n {
            break;
        }

        let here = cost[i];
        let mut relax = |to: usize, mode: usize, value: u32| {
            if value < cost[to][mode] {
                cost[to][mode] = value;
                back[to][mode] = BP_CONTINUE;
            }
        };
        if here[M_NUMERIC] != INF && is_numeric(data[i]) {
            relax(i + 1, M_NUMERIC, here[M_NUMERIC] + STEP[M_NUMERIC]);
        }
        if here[M_ALNUM] != INF && is_alnum(data[i]) {
            relax(i + 1, M_ALNUM, here[M_ALNUM] + STEP[M_ALNUM]);
        }
        if here[M_BYTE] != INF {
            relax(i + 1, M_BYTE, here[M_BYTE] + STEP[M_BYTE]);
        }
        if allow_kanji && here[M_KANJI] != INF && is_kanji_pair(data, i) {
            relax(i + 2, M_KANJI, here[M_KANJI] + STEP[M_KANJI]);
        }
    }

    // Byte mode always reaches the end, so this state is finite and is a valid
    // starting point for the comparison.
    let mut best = M_BYTE;
    let mut best_key = (ceil_bits(cost[n][M_BYTE]), cost[n][M_BYTE]);
    for (j, &end_cost) in cost[n].iter().enumerate() {
        if end_cost == INF {
            continue;
        }
        let key = (ceil_bits(end_cost), end_cost);
        if key < best_key {
            best = j;
            best_key = key;
        }
    }

    let mut segments = Vec::new();
    let mut mode = best;
    let mut at = n;
    let mut end = n;
    loop {
        match back[at][mode] {
            BP_CONTINUE => at -= if mode == M_KANJI { 2 } else { 1 },
            BP_START => {
                if at < end {
                    segments.push(Segment::new(MODES[mode], at, end));
                }
                break;
            }
            from => {
                if at < end {
                    segments.push(Segment::new(MODES[mode], at, end));
                }
                mode = from as usize;
                end = at;
            }
        }
    }
    segments.reverse();
    segments
}

// ---- bit emission ---------------------------------------------------------

/// Appends the character-count indicator to `buf` at `version`.
///
/// Only data modes reach here: [`segments_bits`] returns `None` for
/// [`Mode::Eci`], so version resolution rejects such a segment before any bit is
/// written. The fallback emits no indicator rather than a wrong-width one.
pub fn write_char_count(buf: &mut BitBuffer, mode: Mode, len: usize, version: Version) {
    let ccbits = mode.char_count_bits(version);
    debug_assert!(
        ccbits.is_some(),
        "{mode:?} has no character-count indicator"
    );
    buf.push_bits(len as u32, ccbits.unwrap_or(0));
}

/// Appends the 4-bit mode indicator for `mode` (ISO Table 2).
///
/// The full segment order is `mode indicator`, then char count
/// ([`write_char_count`]), then payload ([`write_payload`]); [`write_segments`]
/// applies all three.
pub fn write_mode_indicator(buf: &mut BitBuffer, mode: Mode) {
    buf.push_bits(mode.indicator() as u32, 4);
}

/// Appends the encoded payload bits for `data` in `mode`.
pub fn write_payload(buf: &mut BitBuffer, mode: Mode, data: &[u8]) {
    match mode {
        Mode::Numeric => {
            let mut chunks = data.chunks_exact(3);
            for c in &mut chunks {
                let v =
                    (c[0] - b'0') as u32 * 100 + (c[1] - b'0') as u32 * 10 + (c[2] - b'0') as u32;
                buf.push_bits(v, 10);
            }
            let rem = chunks.remainder();
            match rem.len() {
                1 => buf.push_bits((rem[0] - b'0') as u32, 4),
                2 => {
                    let v = (rem[0] - b'0') as u32 * 10 + (rem[1] - b'0') as u32;
                    buf.push_bits(v, 7);
                }
                _ => {}
            }
        }
        Mode::Alphanumeric => {
            // Every byte is in the Table 5 set: `mode_supports` gates the forced
            // path and the DP only takes alphanumeric edges on `is_alnum` bytes.
            // The fallback encodes '0', which is in range, so a broken invariant
            // still yields a decodable symbol with wrong characters rather than
            // an out-of-range 11-bit group.
            let value = |b: u8| {
                let v = alnum_value(b);
                debug_assert!(v.is_some(), "byte {b:#04x} is not alphanumeric");
                v.unwrap_or(0)
            };
            let mut chunks = data.chunks_exact(2);
            for c in &mut chunks {
                buf.push_bits(value(c[0]) * 45 + value(c[1]), 11);
            }
            let rem = chunks.remainder();
            if rem.len() == 1 {
                buf.push_bits(value(rem[0]), 6);
            }
        }
        Mode::Byte => {
            for &b in data {
                buf.push_bits(b as u32, 8);
            }
        }
        Mode::Kanji => {
            // Every pair is encodable: `mode_supports` gates the forced path and
            // the DP only takes Kanji edges on `is_kanji_pair` offsets. The
            // fallback is value 0 (Shift-JIS `8140`), a real character, so a
            // broken invariant still yields a decodable 13-bit group.
            for pair in data.chunks_exact(2) {
                let v = kanji_value(pair[0], pair[1]);
                debug_assert!(
                    v.is_some(),
                    "bytes {:#04x} {:#04x} are not a Shift-JIS pair",
                    pair[0],
                    pair[1]
                );
                buf.push_bits(v.unwrap_or(0), 13);
            }
        }
        Mode::Eci => {}
    }
}

/// Appends every segment of `segments` — indicator, character count and payload
/// — reading each segment's bytes from `data`.
pub fn write_segments(buf: &mut BitBuffer, data: &[u8], segments: &[Segment], version: Version) {
    for s in segments {
        write_mode_indicator(buf, s.mode);
        write_char_count(buf, s.mode, s.char_count(), version);
        write_payload(buf, s.mode, &data[s.start..s.end]);
    }
}
