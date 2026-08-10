//! Suite 2 — matrix-level roundtrip: `qr-encode` -> `qr-decode`, no imaging.
//!
//! This is the cheap, exhaustive half of the integration story: every version
//! 1..=40 at every EC level for every content kind, always filled to capacity,
//! so any encoder/decoder disagreement about capacity, padding, block
//! splitting, interleaving, placement or masking shows up immediately.
//!
//! It also closes three decode-review gaps recorded in `handoff/BUILD-LOG.md`:
//!
//! * **KG-1** — multi-block corruption (v5-Q has four RS blocks).
//! * **KG-2** — placement geometry validated independently, by walking the
//!   symbol with our own zigzag and comparing against ISO's published codeword
//!   stream rather than against the encoder's own reader.
//! * **KG-3** — the ISO/IEC 18004:2024 Annex I error-correction codewords are
//!   pinned to their literal published values.

mod common;

use common::{
    ALL_CONTENT, ALL_EC, Content, codewords_from_matrix, corrupt_codeword, data_block_indices,
    data_module_order, ec_label, max_chars, payload, ver,
};
use qr_core::{EcLevel, ec_blocks, reed_solomon::rs_encode};
use qr_decode::{DecodeError, decode};
use qr_encode::{Options, encode};

// ---------------------------------------------------------------------------
// Full v1-40 x EC sweep, one test per content kind (so they run in parallel)
// ---------------------------------------------------------------------------

/// Encodes a capacity-filling payload of `kind` at every version/EC pair and
/// asserts a byte-exact decode with the expected metadata.
fn sweep(kind: Content) {
    for n in 1..=40u8 {
        let v = ver(n);
        for ec in ALL_EC {
            let len = max_chars(kind.mode(), v, ec);
            if len == 0 {
                continue;
            }
            let data = payload(kind, len);
            let mut opts = Options::new(ec);
            opts.version = Some(v);
            let matrix = encode(&data, opts).unwrap_or_else(|e| {
                panic!("v{n}-{} {}: encode failed: {e}", ec_label(ec), kind.label())
            });
            assert_eq!(
                matrix.size(),
                v.size(),
                "v{n}-{} {}: matrix side",
                ec_label(ec),
                kind.label()
            );

            let decoded = decode(&matrix).unwrap_or_else(|e| {
                panic!("v{n}-{} {}: decode failed: {e}", ec_label(ec), kind.label())
            });
            assert_eq!(
                decoded.bytes,
                data,
                "v{n}-{} {}: payload mismatch ({} bytes)",
                ec_label(ec),
                kind.label(),
                len
            );
            assert_eq!(decoded.version.number(), n, "version readback");
            assert_eq!(decoded.ec, ec, "ec readback");
            assert!(decoded.mask <= 7, "mask readback in range");
            assert_eq!(
                decoded.errors_corrected, 0,
                "clean symbol needs no correction"
            );
        }
    }
}

#[test]
fn sweep_numeric_v1_to_v40() {
    sweep(Content::Numeric);
}

#[test]
fn sweep_alphanumeric_v1_to_v40() {
    sweep(Content::Alphanumeric);
}

#[test]
fn sweep_binary_v1_to_v40() {
    sweep(Content::Binary);
}

#[test]
fn sweep_utf8_v1_to_v40() {
    sweep(Content::Utf8);
}

/// Short payloads exercise the terminator + `0xEC`/`0x11` pad path, which the
/// capacity-filling sweep never reaches. Every version and every EC level is
/// covered; the content kind rotates so all four are exercised across the grid
/// without quadrupling the (unoptimised) runtime.
#[test]
fn short_payloads_pad_correctly() {
    for n in 1..=40u8 {
        let v = ver(n);
        for (i, ec) in ALL_EC.into_iter().enumerate() {
            let kind = ALL_CONTENT[(n as usize + i) % ALL_CONTENT.len()];
            let data = payload(kind, 3.min(max_chars(kind.mode(), v, ec)));
            if data.is_empty() {
                continue;
            }
            let mut opts = Options::new(ec);
            opts.version = Some(v);
            let m = encode(&data, opts).expect("encode short payload");
            let d = decode(&m).expect("decode short payload");
            assert_eq!(
                d.bytes,
                data,
                "v{n}-{} {}: short payload mismatch",
                ec_label(ec),
                kind.label()
            );
        }
    }
}

/// Every explicit mask must survive the roundtrip and be reported back.
#[test]
fn all_masks_roundtrip_at_matrix_level() {
    for n in [1u8, 3, 7, 14, 27, 40] {
        let v = ver(n);
        for ec in ALL_EC {
            let data = payload(
                Content::Binary,
                24.min(max_chars(qr_core::Mode::Byte, v, ec)),
            );
            for mask in 0..8u8 {
                let mut opts = Options::new(ec);
                opts.version = Some(v);
                opts.mask = Some(mask);
                let m = encode(&data, opts).expect("encode with forced mask");
                let d = decode(&m).expect("decode with forced mask");
                assert_eq!(d.mask, mask, "v{n}-{} mask {mask}", ec_label(ec));
                assert_eq!(d.bytes, data, "v{n}-{} mask {mask} payload", ec_label(ec));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ISO/IEC 18004:2024 Annex I — pinned reference symbol (KG-2, KG-3)
// ---------------------------------------------------------------------------

/// Annex I.2.2: the 16 version 1-M data codewords for "01234567".
const ANNEX_I_DATA: [u8; 16] = [
    0x10, 0x20, 0x0C, 0x56, 0x61, 0x80, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11,
];

/// Annex I.2.3: the 10 error-correction codewords, transcribed from the binary
/// listing in the 2024 edition of the standard (page 89/90):
/// `10100101 00100100 11010100 11000001 11101101 00110110 11000111 10000111
/// 00101100 01010101`.
const ANNEX_I_EC: [u8; 10] = [0xA5, 0x24, 0xD4, 0xC1, 0xED, 0x36, 0xC7, 0x87, 0x2C, 0x55];

/// KG-3: the EC codewords are pinned to the literal published values, not
/// merely to whatever `rs_encode` happens to produce.
#[test]
fn annex_i_ec_codewords_match_published_values() {
    let ec = rs_encode(&ANNEX_I_DATA, 10);
    assert_eq!(
        ec.as_slice(),
        &ANNEX_I_EC,
        "rs_encode disagrees with ISO/IEC 18004:2024 Annex I.2.3"
    );
}

/// KG-2: walk the finished symbol with an independently written zigzag, undo
/// the mask, and check the recovered codeword stream against Annex I. This
/// validates placement geometry without going through the decoder's own reader.
#[test]
fn annex_i_symbol_placement_matches_published_stream() {
    let m = encode(b"01234567", Options::new(EcLevel::M)).expect("encode Annex I payload");
    assert_eq!(m.size(), 21, "Annex I symbol is version 1");

    let d = decode(&m).expect("decode Annex I symbol");
    assert_eq!(d.mask, 2, "Annex I selects data mask 010");
    assert_eq!(d.ec, EcLevel::M);
    assert_eq!(d.bytes, b"01234567");

    let mut expected = ANNEX_I_DATA.to_vec();
    expected.extend_from_slice(&ANNEX_I_EC);

    let got = codewords_from_matrix(&m, ver(1), 2);
    assert_eq!(
        got.len(),
        26,
        "version 1 holds exactly 26 codewords in its data region"
    );
    assert_eq!(
        got, expected,
        "module placement disagrees with ISO Annex I codeword stream"
    );
}

// ---------------------------------------------------------------------------
// Multi-block Reed-Solomon recovery (KG-1)
// ---------------------------------------------------------------------------

/// Version 5-Q is the smallest configuration with four RS blocks split across
/// two groups (2x15 + 2x16 data, 18 EC each), so a round-robin of corrupted
/// interleaved codewords lands errors in every block at once.
#[test]
fn multi_block_corruption_is_corrected_v5_q() {
    let v = ver(5);
    let blocks = ec_blocks(v, EcLevel::Q);
    assert!(
        blocks.total_blocks() >= 4,
        "v5-Q should have >= 4 RS blocks, got {}",
        blocks.total_blocks()
    );
    let t = blocks.ec_per_block as usize / 2;

    let data = payload(
        Content::Binary,
        max_chars(qr_core::Mode::Byte, v, EcLevel::Q),
    );
    let mut opts = Options::new(EcLevel::Q);
    opts.version = Some(v);
    let mut m = encode(&data, opts).expect("encode v5-Q");

    let order = data_module_order(v);
    let block_of = data_block_indices(blocks);

    // Corrupt the first three interleaving rounds: with four blocks that is
    // three byte errors per block, well inside t = 9.
    let corrupt_count = blocks.total_blocks() as usize * 3;
    let mut per_block = vec![0usize; blocks.total_blocks() as usize];
    for i in 0..corrupt_count {
        per_block[block_of[i]] += 1;
        corrupt_codeword(&mut m, &order, i);
    }
    assert!(
        per_block.iter().filter(|&&c| c > 0).count() >= 2,
        "corruption must span at least two blocks, hit {per_block:?}"
    );
    assert!(
        per_block.iter().all(|&c| c <= t),
        "per-block errors {per_block:?} must stay within t = {t}"
    );

    let d = decode(&m).expect("v5-Q multi-block corruption should be recoverable");
    assert_eq!(d.bytes, data, "payload must survive multi-block correction");
    assert_eq!(
        d.errors_corrected,
        corrupt_count,
        "expected {corrupt_count} corrected codewords across {} blocks",
        blocks.total_blocks()
    );
}

/// Corruption in the EC region is just as correctable as corruption in the data
/// region — the decoder must not special-case it.
#[test]
fn ec_region_corruption_is_corrected_v5_q() {
    let v = ver(5);
    let blocks = ec_blocks(v, EcLevel::Q);
    let data = payload(Content::Alphanumeric, 20);
    let mut opts = Options::new(EcLevel::Q);
    opts.version = Some(v);
    let mut m = encode(&data, opts).expect("encode v5-Q");

    let order = data_module_order(v);
    let first_ec = blocks.total_data_codewords() as usize;
    let corrupt_count = blocks.total_blocks() as usize * 2;
    for i in 0..corrupt_count {
        corrupt_codeword(&mut m, &order, first_ec + i);
    }

    let d = decode(&m).expect("EC-region corruption should be recoverable");
    assert_eq!(d.bytes, data);
    assert_eq!(d.errors_corrected, corrupt_count);
}

/// Past `t` errors in a single block the decoder must fail cleanly, never
/// return a wrong payload and never panic.
#[test]
fn beyond_correction_capacity_fails_cleanly_v5_q() {
    let v = ver(5);
    let blocks = ec_blocks(v, EcLevel::Q);
    let t = blocks.ec_per_block as usize / 2;

    let data = payload(
        Content::Binary,
        max_chars(qr_core::Mode::Byte, v, EcLevel::Q),
    );
    let mut opts = Options::new(EcLevel::Q);
    opts.version = Some(v);
    let mut m = encode(&data, opts).expect("encode v5-Q");

    let order = data_module_order(v);
    let block_of = data_block_indices(blocks);

    // Every corruption targets block 0, so it alone blows past t.
    let targets: Vec<usize> = block_of
        .iter()
        .enumerate()
        .filter(|&(_, &b)| b == 0)
        .map(|(i, _)| i)
        .take(t + 1)
        .collect();
    assert_eq!(
        targets.len(),
        t + 1,
        "block 0 must hold more than t codewords"
    );
    for i in targets {
        corrupt_codeword(&mut m, &order, i);
    }

    match decode(&m) {
        Ok(d) => panic!("expected failure, decoded {:?}", d.bytes),
        Err(e) => assert!(
            matches!(e, DecodeError::BlockCorrection | DecodeError::Malformed),
            "unexpected error kind: {e:?}"
        ),
    }
}
