//! End-to-end decode tests.
//!
//! Fixtures are hand-built from `qr-core` primitives (this crate cannot depend
//! on `qr-encode`, which is built concurrently). The fixture builder is the
//! exact inverse of the decoder's reader, so a matrix built here and then
//! decoded exercises the whole extract → de-interleave → RS → parse pipeline.
//!
//! Only the modules the decoder actually reads are populated (format/version
//! information and the masked data region); finder/timing/alignment patterns
//! are skipped by the decoder via `is_function_module`, so their values are
//! irrelevant to these tests.

use qr_core::bch::{format_info_encode, version_info_encode};
use qr_core::reed_solomon::rs_encode;
use qr_core::{BitMatrix, EcLevel, Mode, Version, ec_blocks, is_function_module, mask_bit};

use qr_decode::{DecodeError, decode};

// ----- fixture construction -------------------------------------------------

/// Places the 15-bit format information into both copies (inverse of the
/// decoder's `read_format_info`).
fn place_format(m: &mut BitMatrix, size: u32, fmt: u16) {
    let bit = |i: u32| (fmt >> i) & 1 == 1;
    for i in 0..6u32 {
        m.set(8, i, bit(i));
    }
    m.set(8, 7, bit(6));
    m.set(8, 8, bit(7));
    m.set(7, 8, bit(8));
    for i in 9..15u32 {
        m.set(14 - i, 8, bit(i));
    }
    for i in 0..8u32 {
        m.set(size - 1 - i, 8, bit(i));
    }
    for i in 8..15u32 {
        m.set(8, size - 15 + i, bit(i));
    }
}

/// Places the 18-bit version information into both copies.
fn place_version(m: &mut BitMatrix, size: u32, info: u32) {
    for i in 0..18u32 {
        let b = (info >> i) & 1 == 1;
        let a = size - 11 + i % 3;
        let row = i / 3;
        m.set(a, row, b);
        m.set(row, a, b);
    }
}

/// Places the interleaved codeword `stream` into the data region using the
/// same zig-zag walk as the decoder, applying the mask on the way in.
fn place_data(m: &mut BitMatrix, version: Version, mask: u8, stream: &[u8]) {
    let size = version.size();
    let total_bits = stream.len() * 8;
    let mut bit = 0usize;
    let mut right = size as i64 - 1;
    while right >= 1 {
        if right == 6 {
            right = 5;
        }
        for vert in 0..size {
            for j in 0..2i64 {
                let x = (right - j) as u32;
                let upward = (right + 1) & 2 == 0;
                let y = if upward { size - 1 - vert } else { vert };
                if !is_function_module(version, x, y) && bit < total_bits {
                    let data_bit = (stream[bit >> 3] >> (7 - (bit & 7))) & 1 == 1;
                    m.set(x, y, data_bit ^ mask_bit(mask, x, y));
                    bit += 1;
                }
            }
        }
        right -= 2;
    }
}

/// Builds a full symbol matrix from a ready interleaved codeword stream.
fn build_matrix(version: Version, ec: EcLevel, mask: u8, stream: &[u8]) -> BitMatrix {
    let size = version.size();
    let mut m = BitMatrix::new(size);
    place_format(&mut m, size, format_info_encode(ec, mask));
    if version.number() >= 7 {
        place_version(&mut m, size, version_info_encode(version.number()).unwrap());
    }
    place_data(&mut m, version, mask, stream);
    m
}

// ----- test-side encoder (byte mode) ---------------------------------------

/// Minimal MSB-first bit writer for building test payloads.
struct BitWriter {
    bytes: Vec<u8>,
    nbits: usize,
}

impl BitWriter {
    fn new() -> BitWriter {
        BitWriter {
            bytes: Vec::new(),
            nbits: 0,
        }
    }
    fn push(&mut self, value: u32, n: u32) {
        for k in (0..n).rev() {
            if self.nbits.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if (value >> k) & 1 == 1 {
                let idx = self.nbits / 8;
                self.bytes[idx] |= 1 << (7 - (self.nbits % 8));
            }
            self.nbits += 1;
        }
    }
}

/// Encodes `payload` as a single byte-mode segment and interleaves it into the
/// codeword stream for `(version, ec)`.
fn encode_byte(version: Version, ec: EcLevel, payload: &[u8]) -> Vec<u8> {
    let bl = ec_blocks(version, ec);
    let total_data = bl.total_data_codewords() as usize;

    let cap_bits = total_data * 8;
    let mut w = BitWriter::new();
    w.push(0b0100, 4); // byte mode
    let cc = Mode::Byte.char_count_bits(version).unwrap() as u32;
    w.push(payload.len() as u32, cc);
    for &b in payload {
        w.push(b as u32, 8);
    }
    // Terminator (up to 4 zero bits), then zero-fill to a byte boundary.
    let term = core::cmp::min(4, cap_bits - w.nbits);
    w.push(0, term as u32);
    while !w.nbits.is_multiple_of(8) {
        w.push(0, 1);
    }

    // Pad bytes 0xEC / 0x11 fill the remaining data capacity.
    let mut data = w.bytes;
    assert!(data.len() <= total_data, "payload too large for version");
    let pad = [0xEC_u8, 0x11];
    let mut i = 0;
    while data.len() < total_data {
        data.push(pad[i]);
        i ^= 1;
    }

    interleave(version, ec, &data)
}

/// Splits `data` into RS blocks, appends EC and interleaves (inverse of the
/// decoder's `deinterleave_and_correct`).
fn interleave(version: Version, ec: EcLevel, data: &[u8]) -> Vec<u8> {
    let bl = ec_blocks(version, ec);
    let g1_blocks = bl.group1_blocks as usize;
    let g1_data = bl.group1_data as usize;
    let g2_data = bl.group2_data as usize;
    let n_blocks = bl.total_blocks() as usize;
    let ecc = bl.ec_per_block as usize;
    let data_len = |b: usize| if b < g1_blocks { g1_data } else { g2_data };

    let mut d_blocks: Vec<Vec<u8>> = Vec::with_capacity(n_blocks);
    let mut e_blocks: Vec<Vec<u8>> = Vec::with_capacity(n_blocks);
    let mut off = 0usize;
    for b in 0..n_blocks {
        let dl = data_len(b);
        let d = data[off..off + dl].to_vec();
        off += dl;
        e_blocks.push(rs_encode(&d, ecc));
        d_blocks.push(d);
    }

    let max_data = g1_data.max(g2_data);
    let mut stream = Vec::with_capacity(version.total_codewords() as usize);
    for i in 0..max_data {
        for (b, block) in d_blocks.iter().enumerate() {
            if i < data_len(b) {
                stream.push(block[i]);
            }
        }
    }
    for i in 0..ecc {
        for block in &e_blocks {
            stream.push(block[i]);
        }
    }
    stream
}

// ----- tests ----------------------------------------------------------------

/// ISO/IEC 18004 Annex I vector: version 1-M, numeric "01234567".
/// Data codewords are the exact Annex I values; EC is derived with `rs_encode`.
fn iso_v1m_stream() -> Vec<u8> {
    let data: [u8; 16] = [
        0x10, 0x20, 0x0C, 0x56, 0x61, 0x80, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11, 0xEC,
        0x11,
    ];
    let mut stream = data.to_vec();
    stream.extend_from_slice(&rs_encode(&data, 10));
    stream
}

#[test]
fn decodes_iso_annex_i_vector() {
    let v = Version::new(1).unwrap();
    let m = build_matrix(v, EcLevel::M, 2, &iso_v1m_stream());

    let d = decode(&m).expect("decode");
    assert_eq!(d.bytes, b"01234567");
    assert_eq!(d.text.as_deref(), Some("01234567"));
    assert_eq!(d.version, v);
    assert_eq!(d.ec, EcLevel::M);
    assert_eq!(d.mask, 2);
    assert_eq!(d.errors_corrected, 0);
}

#[test]
fn corrects_corrupted_codewords() {
    let v = Version::new(1).unwrap();
    let mut m = build_matrix(v, EcLevel::M, 2, &iso_v1m_stream());

    // Flip a handful of data modules (well within v1-M's 5-codeword capacity).
    // Use interior data-region coordinates that are not function modules.
    let mut flipped = 0;
    'outer: for y in 9..v.size() - 9 {
        for x in 9..v.size() - 9 {
            if !is_function_module(v, x, y) {
                let cur = m.get(x, y);
                m.set(x, y, !cur);
                flipped += 1;
                if flipped == 6 {
                    break 'outer;
                }
            }
        }
    }

    let d = decode(&m).expect("decode with correction");
    assert_eq!(d.bytes, b"01234567");
    assert!(d.errors_corrected > 0);
}

#[test]
fn rejects_uncorrectable_matrix() {
    let v = Version::new(1).unwrap();
    let mut m = build_matrix(v, EcLevel::M, 2, &iso_v1m_stream());

    // Corrupt far more data modules than can be corrected.
    let mut flipped = 0;
    'outer: for y in 9..v.size() {
        for x in 0..v.size() {
            if !is_function_module(v, x, y) {
                let cur = m.get(x, y);
                m.set(x, y, !cur);
                flipped += 1;
                if flipped == 90 {
                    break 'outer;
                }
            }
        }
    }

    assert_eq!(decode(&m), Err(DecodeError::BlockCorrection));
}

#[test]
fn rejects_invalid_size() {
    // Side length 20 is not 17 + 4*version.
    let m = BitMatrix::new(20);
    assert_eq!(decode(&m), Err(DecodeError::InvalidSize));
}

#[test]
fn decodes_mirrored_symbol() {
    let v = Version::new(1).unwrap();
    let m = build_matrix(v, EcLevel::M, 2, &iso_v1m_stream());

    // Reflect across the main diagonal; decode must recover it via the retry.
    let size = m.size();
    let mut mirrored = BitMatrix::new(size);
    for y in 0..size {
        for x in 0..size {
            if m.get(x, y) {
                mirrored.set(y, x, true);
            }
        }
    }

    let d = decode(&mirrored).expect("decode mirrored");
    assert_eq!(d.bytes, b"01234567");
}

#[test]
fn decodes_multiblock_byte_symbol() {
    // v5-Q has two group-1 blocks (15 data) and two group-2 blocks (16 data):
    // exercises the unequal-group de-interleave path.
    let v = Version::new(5).unwrap();
    let payload: Vec<u8> = (0..40u8).map(|i| b'A' + (i % 26)).collect();
    let stream = encode_byte(v, EcLevel::Q, &payload);

    let m = build_matrix(v, EcLevel::Q, 5, &stream);
    let d = decode(&m).expect("decode v5-Q");
    assert_eq!(d.bytes, payload);
    assert_eq!(d.version, v);
    assert_eq!(d.ec, EcLevel::Q);
    assert_eq!(d.mask, 5);
    assert_eq!(d.errors_corrected, 0);
}

#[test]
fn decodes_version_info_symbol() {
    // v7 carries version information; this checks reading it plus the
    // four-block interleave for v7-M.
    let v = Version::new(7).unwrap();
    let payload: Vec<u8> = (0..80u8).map(|i| i.wrapping_mul(7)).collect();
    let stream = encode_byte(v, EcLevel::M, &payload);

    let m = build_matrix(v, EcLevel::M, 3, &stream);
    let d = decode(&m).expect("decode v7-M");
    assert_eq!(d.bytes, payload);
    assert_eq!(d.version, v);
}
