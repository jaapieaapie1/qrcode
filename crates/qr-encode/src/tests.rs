//! Encoder tests, including the ISO/IEC 18004 Annex I reference symbol.

use crate::bits::BitBuffer;
use crate::segment::{self, Segment};
use crate::{Options, encode, matrix};
use alloc::vec;
use alloc::vec::Vec;
use qr_core::{
    BitMatrix, EcLevel, Mode, Version, bch::format_info_decode, ec_blocks, is_function_module,
    reed_solomon::rs_correct,
};

/// The ISO Annex I data codewords for "01234567" at version 1, level M.
const ANNEX_I_DATA: [u8; 16] = [
    0x10, 0x20, 0x0C, 0x56, 0x61, 0x80, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11,
];
/// The matching 10 Reed-Solomon EC codewords (ISO Annex I).
const ANNEX_I_EC: [u8; 10] = [0xA5, 0x24, 0xD4, 0xC1, 0xED, 0x36, 0xC7, 0x87, 0x2C, 0x55];

// ---- bit-stream / codeword assembly --------------------------------------

#[test]
fn annex_i_data_codewords() {
    let v = Version::new(1).unwrap();
    let segs = [Segment::new(Mode::Numeric, 0, 8)];
    let cw = crate::build_data_codewords(b"01234567", &segs, None, v, EcLevel::M);
    assert_eq!(cw, ANNEX_I_DATA);
}

#[test]
fn annex_i_interleave_produces_reference_ec() {
    let v = Version::new(1).unwrap();
    let blocks = ec_blocks(v, EcLevel::M);
    let segs = [Segment::new(Mode::Numeric, 0, 8)];
    let cw = crate::build_data_codewords(b"01234567", &segs, None, v, EcLevel::M);
    let inter = matrix::interleave(&cw, blocks);
    // v1-M is a single block: 16 data followed by 10 EC.
    assert_eq!(inter.len(), 26);
    assert_eq!(&inter[..16], &ANNEX_I_DATA);
    assert_eq!(&inter[16..], &ANNEX_I_EC);
}

// ---- full-symbol round trip ----------------------------------------------

/// Reads the first copy of the 15-bit format information from a symbol.
fn read_format_bits(m: &BitMatrix) -> u16 {
    let get = |x: u32, y: u32| if m.get(x, y) { 1u16 } else { 0 };
    let mut bits = 0u16;
    for i in 0..6u16 {
        bits |= get(8, i as u32) << i;
    }
    bits |= get(8, 7) << 6;
    bits |= get(8, 8) << 7;
    bits |= get(7, 8) << 8;
    for i in 9..15u16 {
        bits |= get(14 - i as u32, 8) << i;
    }
    bits
}

/// Extracts the interleaved codeword stream from an unmasked symbol using the
/// same zigzag order the encoder writes with.
fn extract_interleaved(m: &BitMatrix, version: Version, n_codewords: usize) -> Vec<u8> {
    let size = version.size();
    let total_bits = n_codewords * 8;
    let mut out = vec![0u8; n_codewords];
    let mut bit_index = 0usize;
    let mut right = size as i32 - 1;
    while right >= 1 {
        if right == 6 {
            right = 5;
        }
        for vert in 0..size {
            for j in 0..2 {
                let x = (right - j) as u32;
                let upward = ((right + 1) & 2) == 0;
                let y = if upward { size - 1 - vert } else { vert };
                if is_function_module(version, x, y) {
                    continue;
                }
                if bit_index < total_bits {
                    if m.get(x, y) {
                        out[bit_index >> 3] |= 1 << (7 - (bit_index & 7));
                    }
                    bit_index += 1;
                }
            }
        }
        right -= 2;
    }
    out
}

#[test]
fn annex_i_full_symbol_roundtrip() {
    let m = encode(b"01234567", Options::new(EcLevel::M)).unwrap();
    let v = Version::new(1).unwrap();
    assert_eq!(m.size(), 21);

    // Format info must decode to level M and a valid mask via the core BCH path.
    let (ec, mask) = format_info_decode(read_format_bits(&m)).expect("format decodes");
    assert_eq!(ec, EcLevel::M);
    assert!(mask < 8);

    // Undo the mask (XOR is its own inverse), then read the codewords back.
    let mut unmasked = m.clone();
    matrix::reference::apply_mask(&mut unmasked, v, mask);
    let inter = extract_interleaved(&unmasked, v, 26);
    assert_eq!(&inter[..16], &ANNEX_I_DATA);
    assert_eq!(&inter[16..], &ANNEX_I_EC);
}

#[test]
fn annex_i_selected_mask_is_two() {
    // ISO Annex I applies the 7.8.3 penalty and selects data mask 010 (2) for
    // this symbol; the auto-selector must reproduce that choice.
    let m = encode(b"01234567", Options::new(EcLevel::M)).unwrap();
    let (_, mask) = format_info_decode(read_format_bits(&m)).unwrap();
    assert_eq!(mask, 2);
}

// ---- forced options ------------------------------------------------------

#[test]
fn forced_mask_is_honoured_and_verifiable() {
    for forced in 0..8u8 {
        let opts = Options {
            ec: EcLevel::Q,
            version: None,
            mask: Some(forced),
            mode: None,
            eci: None,
            assume_shift_jis: false,
            single_segment: false,
            boost_ec: false,
        };
        let m = encode(b"HELLO WORLD", opts).unwrap();
        let (ec, mask) = format_info_decode(read_format_bits(&m)).unwrap();
        assert_eq!(ec, EcLevel::Q);
        assert_eq!(mask, forced);
    }
}

#[test]
fn byte_mode_roundtrips_through_ec_correction() {
    // Encode arbitrary bytes, then confirm every RS block corrects cleanly
    // (zero errors) once the mask is removed and the stream deinterleaved.
    let payload: Vec<u8> = (0..40u8).collect();
    let opts = Options {
        ec: EcLevel::H,
        version: Some(Version::new(5).unwrap()),
        mask: None,
        mode: Some(Mode::Byte),
        eci: None,
        assume_shift_jis: false,
        single_segment: false,
        boost_ec: false,
    };
    let m = encode(&payload, opts).unwrap();
    let v = Version::new(5).unwrap();
    let blocks = ec_blocks(v, EcLevel::H);

    let (_, mask) = format_info_decode(read_format_bits(&m)).unwrap();
    let mut unmasked = m.clone();
    matrix::reference::apply_mask(&mut unmasked, v, mask);

    let total = blocks.total_codewords() as usize;
    let inter = extract_interleaved(&unmasked, v, total);

    // Deinterleave into per-block (data||ec) codewords, then RS-correct each.
    let mut deint = deinterleave(&inter, blocks);
    for blk in &mut deint {
        let corrected = rs_correct(blk, blocks.ec_per_block as usize).expect("block corrects");
        assert_eq!(corrected, 0, "a clean symbol needs no corrections");
    }
}

/// Inverse of `matrix::interleave`: splits the interleaved stream back into
/// per-block `data || ec` codeword vectors.
fn deinterleave(inter: &[u8], blocks: qr_core::EcBlocks) -> Vec<Vec<u8>> {
    let g1b = blocks.group1_blocks as usize;
    let g1d = blocks.group1_data as usize;
    let g2b = blocks.group2_blocks as usize;
    let g2d = blocks.group2_data as usize;
    let ec_len = blocks.ec_per_block as usize;
    let n_blocks = g1b + g2b;

    let block_data_len = |b: usize| if b < g1b { g1d } else { g2d };
    let mut out: Vec<Vec<u8>> = (0..n_blocks).map(|_| Vec::new()).collect();

    let max_data = g1d.max(g2d);
    let mut idx = 0usize;
    for i in 0..max_data {
        for (b, blk) in out.iter_mut().enumerate() {
            if i < block_data_len(b) {
                blk.push(inter[idx]);
                idx += 1;
            }
        }
    }
    // EC codewords occupy the tail, interleaved column-by-column across blocks.
    let ec_start = idx;
    for i in 0..ec_len {
        for (b, blk) in out.iter_mut().enumerate() {
            blk.push(inter[ec_start + i * n_blocks + b]);
        }
    }
    out
}

// ---- penalty rules -------------------------------------------------------

#[test]
fn n1_run_penalty_counts_long_runs() {
    // A run of exactly five same-colour modules scores 3; each extra adds 1.
    let five = [true, true, true, true, true, false];
    assert_eq!(
        matrix::reference::line_run_penalty(|i| five[i as usize], 6),
        3
    );
    let seven = [true; 7];
    assert_eq!(
        matrix::reference::line_run_penalty(|i| seven[i as usize], 7),
        5
    );
    let short = [true, true, false, true, true, false, true];
    assert_eq!(
        matrix::reference::line_run_penalty(|i| short[i as usize], 7),
        0
    );
}

#[test]
fn n3_finder_penalty_detects_pattern() {
    // A 1:1:3:1:1 pattern with a 4-wide light area scores 40 for its existence
    // (once, per ISO NOTE 3 — not per side).
    let line = [
        false, false, false, false, true, false, true, true, true, false, true,
    ];
    assert_eq!(
        matrix::reference::finder_penalty(|i| line[i as usize], line.len() as u32),
        40
    );
    // A line with no finder-like pattern scores nothing.
    let plain = [false, true, false, true, false, true, false, true, false];
    assert_eq!(
        matrix::reference::finder_penalty(|i| plain[i as usize], plain.len() as u32),
        0
    );
}

// ---- smoke across versions ----------------------------------------------

#[test]
fn smoke_encode_all_versions() {
    // A payload sized to land in each version, checking the symbol is
    // well-formed (correct size, valid format info) end to end.
    for n in 1..=40u8 {
        let v = Version::new(n).unwrap();
        let cap = ec_blocks(v, EcLevel::L).total_data_codewords() as usize;
        // Fill roughly to half capacity with byte data.
        let payload: Vec<u8> = (0..(cap / 2).max(1)).map(|i| (i % 251) as u8).collect();
        let opts = Options {
            ec: EcLevel::L,
            version: Some(v),
            mask: None,
            mode: Some(Mode::Byte),
            eci: None,
            assume_shift_jis: false,
            single_segment: false,
            boost_ec: false,
        };
        let m = encode(&payload, opts).unwrap_or_else(|e| panic!("v{n}: {e}"));
        assert_eq!(m.size(), v.size());
        let (ec, _) = format_info_decode(read_format_bits(&m)).expect("format decodes");
        assert_eq!(ec, EcLevel::L);
    }
}

// ---- Kanji mode (ISO 7.4.6) ----------------------------------------------

#[test]
fn iso_7_4_6_kanji_example_values() {
    // ISO/IEC 18004 §7.4.6 EXAMPLE, verbatim:
    //   "点" = 935F: 935F - 8140 = 121F; 12 x C0 = D80; D80 + 1F = D9F
    //   "茗" = E4AA: E4AA - C140 = 236A; 23 x C0 = 1A40; 1A40 + 6A = 1AAA
    assert_eq!(segment::kanji_value(0x93, 0x5F), Some(0x0D9F));
    assert_eq!(segment::kanji_value(0xE4, 0xAA), Some(0x1AAA));
}

#[test]
fn iso_7_4_6_kanji_example_bits() {
    // The same example as a bit stream: 0 1101 1001 1111 then 1 1010 1010 1010,
    // i.e. 26 bits, zero-padded to 0x6C 0xFE 0xAA 0x80.
    let mut buf = BitBuffer::new();
    segment::write_payload(&mut buf, Mode::Kanji, &[0x93, 0x5F, 0xE4, 0xAA]);
    assert_eq!(buf.len(), 26);
    assert_eq!(buf.into_bytes(), vec![0x6C, 0xFE, 0xAA, 0x80]);
}

#[test]
fn kanji_range_boundaries_are_exact() {
    // The two ISO ranges, at their edges.
    assert_eq!(segment::kanji_value(0x81, 0x40), Some(0));
    assert_eq!(segment::kanji_value(0x9F, 0xFC), Some(0x1E * 0xC0 + 0xBC));
    assert_eq!(segment::kanji_value(0xE0, 0x40), Some(0x1F * 0xC0));
    // E0,40 is the first code of the second range: 0x1F00 in difference space.
    assert_eq!(segment::kanji_value(0xEB, 0xBF), Some(0x1FFF));
    // Outside both ranges, or a low byte that would not round-trip.
    assert_eq!(segment::kanji_value(0x81, 0x3F), None);
    assert_eq!(segment::kanji_value(0xA0, 0x41), None);
    assert_eq!(segment::kanji_value(0xEB, 0xC0), None);
    assert_eq!(segment::kanji_value(0xEC, 0x40), None);
    assert_eq!(segment::kanji_value(0xFF, 0xFE), None);
}

#[test]
fn kanji_pairs_segment_as_kanji_only_when_asserted() {
    let v = Version::new(1).unwrap();
    let sjis = [0x93u8, 0x5F, 0xE4, 0xAA];

    // Opted in: 4 (indicator) + 8 (v1 Kanji count) + 2 x 13 = 38 bits, against
    // 4 + 8 + 4 x 8 = 44 for byte mode.
    let segs = segment::optimal_segments(&sjis, v, true);
    assert_eq!(segs, vec![Segment::new(Mode::Kanji, 0, 4)]);
    assert_eq!(segs[0].char_count(), 2);
    assert_eq!(segment::segments_bits(&segs, v), Some(38));

    // Default: the same bytes stay in byte mode, costing the extra 6 bits.
    let segs = segment::optimal_segments(&sjis, v, false);
    assert_eq!(segs, vec![Segment::new(Mode::Byte, 0, 4)]);
    assert_eq!(segment::segments_bits(&segs, v), Some(44));
}

// ---- ECI header (ISO 7.4.3.2, Table 4) -----------------------------------

/// Emits just an ECI header and returns the packed bytes plus the bit length.
fn eci_stream(value: u32) -> (Vec<u8>, usize) {
    let mut buf = BitBuffer::new();
    segment::write_eci(&mut buf, value);
    let len = buf.len();
    (buf.into_bytes(), len)
}

#[test]
fn eci_designator_one_codeword() {
    // ECI 26 (UTF-8): 0111 then 0bbbbbbb = 0011010.
    assert_eq!(segment::eci_bits(26), Some(12));
    assert_eq!(eci_stream(26), (vec![0x71, 0xA0], 12));
    // 127 is the last one-codeword assignment.
    assert_eq!(segment::eci_bits(127), Some(12));
}

#[test]
fn eci_designator_two_codewords() {
    // ECI 128: 0111 then 10bbbbbb bbbbbbbb = 1000000010000000.
    assert_eq!(segment::eci_bits(128), Some(20));
    assert_eq!(eci_stream(128), (vec![0x78, 0x08, 0x00], 20));
    assert_eq!(segment::eci_bits(16_383), Some(20));
}

#[test]
fn eci_designator_three_codewords() {
    // ECI 999999: 0111 then 110bbbbb bbbbbbbb bbbbbbbb = 110 0 1111 0100 0010
    // 0011 1111 (999999 = 0xF423F).
    assert_eq!(segment::eci_bits(16_384), Some(28));
    assert_eq!(segment::eci_bits(999_999), Some(28));
    assert_eq!(eci_stream(999_999), (vec![0x7C, 0xF4, 0x23, 0xF0], 28));
}

#[test]
fn eci_out_of_range_is_rejected() {
    assert_eq!(segment::eci_bits(1_000_000), None);
    let opts = Options {
        ec: EcLevel::M,
        version: None,
        mask: None,
        mode: None,
        eci: Some(1_000_000),
        assume_shift_jis: false,
        single_segment: false,
        boost_ec: false,
    };
    assert_eq!(encode(b"123", opts), Err(crate::EncodeError::InvalidEci));
}

#[test]
fn eci_header_precedes_the_data_segments() {
    let v = Version::new(1).unwrap();
    let segs = [Segment::new(Mode::Numeric, 0, 8)];
    let cw = crate::build_data_codewords(b"01234567", &segs, Some(26), v, EcLevel::M);
    // 0111 00011010 then the Annex I stream (0001 0000001000 ...), i.e. the
    // reference codewords shifted right by the 12-bit ECI header.
    let mut expect = BitBuffer::new();
    segment::write_eci(&mut expect, 26);
    segment::write_segments(&mut expect, b"01234567", &segs, v);
    let head = expect.into_bytes();
    assert_eq!(&cw[..head.len() - 1], &head[..head.len() - 1]);
    assert_eq!(cw[0], 0x71);
}

// ---- optimal segmentation -------------------------------------------------

/// The 45-character alphanumeric set (ISO Table 5).
const ALNUM_SET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";

/// Total bits under the pre-DP rule: one segment in the most compact mode that
/// can hold every byte (numeric < alphanumeric < byte).
fn single_mode_bits(data: &[u8], version: Version) -> usize {
    let mode = if !data.is_empty() && data.iter().all(|b| b.is_ascii_digit()) {
        Mode::Numeric
    } else if !data.is_empty() && data.iter().all(|b| ALNUM_SET.contains(b)) {
        Mode::Alphanumeric
    } else {
        Mode::Byte
    };
    4 + mode.char_count_bits(version).unwrap() as usize + segment::payload_bits(mode, data.len())
}

fn dp_bits(data: &[u8], version: Version) -> usize {
    let segs = segment::optimal_segments(data, version, false);
    segment::segments_bits(&segs, version).unwrap()
}

/// A deterministic xorshift64 so the property test is reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Every payload is covered exactly once, in order, with a mode that can hold
/// its bytes.
fn assert_well_formed(data: &[u8], segs: &[Segment]) {
    let mut at = 0usize;
    for s in segs {
        assert_eq!(s.start, at, "segments must be contiguous");
        assert!(s.end <= data.len());
        assert!(
            segment::mode_supports(s.mode, &data[s.start..s.end]),
            "{:?} cannot hold {:?}",
            s.mode,
            &data[s.start..s.end]
        );
        at = s.end;
    }
    assert_eq!(at, data.len(), "segments must cover the payload");
}

#[test]
fn dp_never_loses_to_single_mode() {
    // One version per ISO Table 3 group, since that is all segmentation sees.
    let anchors = [1u8, 10, 27].map(|n| Version::new(n).unwrap());
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for trial in 0..600u32 {
        // Bias the alphabet per trial so runs of each mode actually occur.
        let len = (rng.next() % 90) as usize + 1;
        let payload: Vec<u8> = (0..len)
            .map(|_| {
                let r = rng.next();
                match (trial + (r >> 32) as u32) % 5 {
                    0 | 1 => b'0' + (r % 10) as u8,
                    2 => b'A' + (r % 26) as u8,
                    3 => b'a' + (r % 26) as u8,
                    _ => (r % 256) as u8,
                }
            })
            .collect();
        for v in anchors {
            for kanji in [false, true] {
                let segs = segment::optimal_segments(&payload, v, kanji);
                assert_well_formed(&payload, &segs);
                if !kanji {
                    assert!(
                        segs.iter().all(|s| s.mode != Mode::Kanji),
                        "v{} {payload:?}: Kanji segment without opt-in",
                        v.number()
                    );
                }
                let dp = segment::segments_bits(&segs, v).unwrap();
                let single = single_mode_bits(&payload, v);
                assert!(
                    dp <= single,
                    "v{} {payload:?} (kanji={kanji}): DP {dp} bits > single-mode {single} bits",
                    v.number()
                );
            }
        }
    }
}

#[test]
fn pure_payloads_keep_their_single_mode() {
    let v = Version::new(1).unwrap();
    for (data, mode) in [
        (&b"01234567"[..], Mode::Numeric),
        (&b"HELLO WORLD"[..], Mode::Alphanumeric),
        (&b"hello world"[..], Mode::Byte),
    ] {
        let segs = segment::optimal_segments(data, v, false);
        assert_eq!(segs, vec![Segment::new(mode, 0, data.len())], "{data:?}");
        assert_eq!(
            segment::segments_bits(&segs, v).unwrap(),
            single_mode_bits(data, v)
        );
    }
}

#[test]
fn short_digit_run_stays_alphanumeric() {
    // "ABC123456789DEF" is 96 bits as one alphanumeric segment; splitting the
    // nine digits out costs 104. A greedy splitter gets this wrong.
    let v = Version::new(1).unwrap();
    let data = b"ABC123456789DEF";
    assert_eq!(
        segment::optimal_segments(data, v, false),
        vec![Segment::new(Mode::Alphanumeric, 0, data.len())]
    );
    assert_eq!(dp_bits(data, v), 96);
}

#[test]
fn long_digit_run_splits_out_of_alphanumeric() {
    let v = Version::new(1).unwrap();
    let data = b"ABC123456789012345678901234567890DEF";
    let segs = segment::optimal_segments(data, v, false);
    assert_eq!(
        segs,
        vec![
            Segment::new(Mode::Alphanumeric, 0, 3),
            Segment::new(Mode::Numeric, 3, 33),
            Segment::new(Mode::Alphanumeric, 33, 36),
        ]
    );
    assert!(dp_bits(data, v) < single_mode_bits(data, v));
}

#[test]
fn url_with_digits_splits_the_numeric_tail() {
    let v = Version::new(1).unwrap();
    let data = b"http://example.com/1234567890123456789012";
    let segs = segment::optimal_segments(data, v, false);
    assert!(segs.len() > 1, "expected a split, got {segs:?}");
    assert_eq!(segs.last().unwrap().mode, Mode::Numeric);
    assert!(dp_bits(data, v) < single_mode_bits(data, v));
    assert_well_formed(data, &segs);
}

/// True minimum bit count over *every* valid segmentation of `data`, by
/// exhaustive search. Exponential — small inputs only.
///
/// Enumerates each prefix split and each mode that can hold it, which is the
/// same solution space the DP searches, evaluated with `segments_bits` so the
/// comparison cannot inherit a bug from the DP's own cost model.
fn brute_force_bits(data: &[u8], version: Version, allow_kanji: bool) -> usize {
    fn go(
        data: &[u8],
        at: usize,
        version: Version,
        allow_kanji: bool,
        acc: &mut Vec<Segment>,
        best: &mut usize,
    ) {
        if at == data.len() {
            let bits = segment::segments_bits(acc, version).unwrap();
            *best = (*best).min(bits);
            return;
        }
        let modes: &[Mode] = if allow_kanji {
            &[Mode::Numeric, Mode::Alphanumeric, Mode::Byte, Mode::Kanji]
        } else {
            &[Mode::Numeric, Mode::Alphanumeric, Mode::Byte]
        };
        for end in (at + 1)..=data.len() {
            for &mode in modes {
                if !segment::mode_supports(mode, &data[at..end]) {
                    continue;
                }
                acc.push(Segment::new(mode, at, end));
                go(data, end, version, allow_kanji, acc, best);
                acc.pop();
            }
        }
    }
    let mut best = usize::MAX;
    go(data, 0, version, allow_kanji, &mut Vec::new(), &mut best);
    best
}

#[test]
fn dp_matches_brute_force_optimum() {
    // A six-symbol alphabet spanning every mode class: a digit (numeric,
    // alphanumeric, byte), an uppercase letter (alphanumeric, byte), a
    // lowercase letter (byte only) and three Shift-JIS bytes that pair into
    // both ISO Kanji ranges.
    const ALPHABET: [u8; 6] = [b'0', b'A', b'a', 0x93, 0x5F, 0xE4];
    let anchors = [1u8, 10, 27].map(|n| Version::new(n).unwrap());

    for len in 1..=6usize {
        let total = ALPHABET.len().pow(len as u32);
        for encoded in 0..total {
            let mut n = encoded;
            let payload: Vec<u8> = (0..len)
                .map(|_| {
                    let b = ALPHABET[n % ALPHABET.len()];
                    n /= ALPHABET.len();
                    b
                })
                .collect();
            for v in anchors {
                for kanji in [false, true] {
                    let segs = segment::optimal_segments(&payload, v, kanji);
                    let dp = segment::segments_bits(&segs, v).unwrap();
                    let brute = brute_force_bits(&payload, v, kanji);
                    // Both that the DP found the optimum, and that the
                    // backtracked segments actually cost what it reported — a
                    // back-pointer walk that disagreed with the cost array
                    // would slip past a cost-only comparison.
                    assert_eq!(
                        dp,
                        brute,
                        "v{} kanji={kanji} {payload:?}: DP {dp} != optimum {brute} ({segs:?})",
                        v.number()
                    );
                    assert_well_formed(&payload, &segs);
                }
            }
        }
    }
}

#[test]
fn empty_payload_is_one_empty_byte_segment() {
    let v = Version::new(1).unwrap();
    let segs = segment::optimal_segments(b"", v, false);
    assert_eq!(segs, vec![Segment::new(Mode::Byte, 0, 0)]);
    assert_eq!(segment::segments_bits(&segs, v), Some(12));
}

#[test]
fn version_auto_select_is_still_the_smallest_fit() {
    // Walk payload lengths across the v1/v2 and group-0/group-1 boundaries and
    // confirm the chosen version is the first one whose capacity holds the DP
    // bit count.
    for len in [1usize, 10, 17, 18, 30, 100, 200, 300, 400] {
        let payload: Vec<u8> = (0..len).map(|i| b'0' + (i % 10) as u8).collect();
        let m = encode(&payload, Options::new(EcLevel::L)).expect("encode");
        let chosen = (m.size() as u8 - 17) / 4;
        for n in 1..chosen {
            let v = Version::new(n).unwrap();
            let bits = dp_bits(&payload, v);
            assert!(
                bits > crate::data_capacity_bits(v, EcLevel::L),
                "len {len}: v{n} would have fit ({bits} bits) but v{chosen} was chosen"
            );
        }
        let v = Version::new(chosen).unwrap();
        assert!(dp_bits(&payload, v) <= crate::data_capacity_bits(v, EcLevel::L));
    }
}

#[test]
fn too_large_payload_is_rejected() {
    let opts = Options {
        ec: EcLevel::H,
        version: Some(Version::new(1).unwrap()),
        mask: None,
        mode: Some(Mode::Byte),
        eci: None,
        assume_shift_jis: false,
        single_segment: false,
        boost_ec: false,
    };
    let big = vec![0u8; 1000];
    assert!(encode(&big, opts).is_err());
}
