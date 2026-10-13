//! Independent encode -> decode roundtrip cross-check.
//!
//! These tests do NOT use the encoder's own bit-extraction helpers. They feed
//! each finished symbol to `qr_decode::decode`, a separate implementation, so a
//! zigzag/placement/mask/interleave error that is merely *self-consistent*
//! inside qr-encode (and therefore invisible to the in-crate extract-based
//! tests) surfaces here as a decode failure or payload mismatch.
//!
//! Added during review of Step 2a (Richard-E).

use qr_core::{EcLevel, Mode, Version, ec_blocks};
use qr_encode::{Options, encode};

const LEVELS: [EcLevel; 4] = [EcLevel::L, EcLevel::M, EcLevel::Q, EcLevel::H];

/// A shifted or transposed zigzag would corrupt the payload while the encoder's
/// own extract-based tests (which share the shift) still pass. Decoding with the
/// independent decoder catches it.
#[test]
fn byte_payload_roundtrips_every_level_auto_mask() {
    // Distinctive, non-symmetric payload so a shift cannot alias to itself.
    let payload: Vec<u8> = (0u16..64)
        .map(|i| (i.wrapping_mul(37) ^ 0x5A) as u8)
        .collect();
    for ec in LEVELS {
        let m = encode(&payload, Options::new(ec)).expect("encode");
        let decoded = qr_decode::decode(&m).expect("decode");
        assert_eq!(decoded.bytes, payload, "payload mismatch at ec {ec:?}");
        assert_eq!(decoded.ec, ec, "ec mismatch");
    }
}

/// Exercises multi-block interleave (v5-H is a 2-block, 2-group symbol) through
/// the independent decoder, including error-correction counting on a clean read.
#[test]
fn multiblock_symbol_roundtrips() {
    let v5 = Version::new(5).unwrap();
    // Fill v5-H (a 2-group, 4-block symbol) close to capacity.
    let cap = qr_core::ec_blocks(v5, EcLevel::H).total_data_codewords() as usize;
    let payload: Vec<u8> = (0..(cap - 3)).map(|i| (i as u8).wrapping_mul(53)).collect();
    let opts = Options {
        ec: EcLevel::H,
        version: Some(v5),
        mask: None,
        mode: Some(Mode::Byte),
        eci: None,
        assume_shift_jis: false,
        single_segment: false,
        boost_ec: false,
    };
    let m = encode(&payload, opts).expect("encode");
    let decoded = qr_decode::decode(&m).expect("decode");
    assert_eq!(decoded.bytes, payload);
    assert_eq!(decoded.version, Version::new(5).unwrap());
    assert_eq!(
        decoded.errors_corrected, 0,
        "clean symbol needs no correction"
    );
}

/// Every forced mask must produce a symbol the decoder reads back correctly and
/// reports the same mask id — verifying both format-info placement and that the
/// mask XOR is geometrically the inverse the decoder expects.
#[test]
fn all_forced_masks_roundtrip() {
    let payload = b"HELLO WORLD 123";
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
        let m = encode(payload, opts).expect("encode");
        let decoded = qr_decode::decode(&m).unwrap_or_else(|e| panic!("mask {forced}: {e:?}"));
        assert_eq!(decoded.bytes, &payload[..], "mask {forced} payload");
        assert_eq!(decoded.mask, forced, "mask {forced} id");
    }
}

/// Numeric mode roundtrip, and confirmation that auto mask-selection lands on
/// ISO Annex I's mask 2 for the canonical "01234567" v1-M symbol.
#[test]
fn numeric_annex_i_roundtrips_with_mask_two() {
    let m = encode(b"01234567", Options::new(EcLevel::M)).expect("encode");
    let decoded = qr_decode::decode(&m).expect("decode");
    assert_eq!(decoded.bytes, b"01234567");
    assert_eq!(decoded.version, Version::new(1).unwrap());
    assert_eq!(decoded.ec, EcLevel::M);
    assert_eq!(decoded.mask, 2, "ISO Annex I selects data mask 010");
}

/// Version >= 7 exercises the 18-bit version-information blocks; decode reads
/// and cross-checks them, so a mis-oriented version-info placement is caught.
#[test]
fn version_info_symbol_roundtrips() {
    // Size a byte payload to land around v7-v10 and force each of those versions.
    for n in [7u8, 8, 10, 14] {
        let v = Version::new(n).unwrap();
        let cap = qr_core::ec_blocks(v, EcLevel::M).total_data_codewords() as usize;
        let payload: Vec<u8> = (0..(cap - 4).min(cap)).map(|i| (i as u8) ^ 0x33).collect();
        let opts = Options {
            ec: EcLevel::M,
            version: Some(v),
            mask: None,
            mode: Some(Mode::Byte),
            eci: None,
            assume_shift_jis: false,
            single_segment: false,
            boost_ec: false,
        };
        let m = encode(&payload, opts).unwrap_or_else(|e| panic!("v{n}: {e:?}"));
        let decoded = qr_decode::decode(&m).unwrap_or_else(|e| panic!("v{n} decode: {e:?}"));
        assert_eq!(decoded.bytes, payload, "v{n} payload");
        assert_eq!(decoded.version, v, "v{n} version");
    }
}

// ---- edge cases: boundaries and error paths ------------------------------

#[test]
fn empty_data() {
    let m = encode(b"", Options::new(EcLevel::M)).expect("empty encodes");
    let d = qr_decode::decode(&m).expect("empty decodes");
    assert_eq!(d.bytes, b"");
}

#[test]
fn char_count_never_overflows_indicator() {
    // For every version/mode, the max char count admitted by the capacity check
    // must fit the char-count indicator width.
    for n in 1..=40u8 {
        let v = Version::new(n).unwrap();
        for ec in [EcLevel::L, EcLevel::M, EcLevel::Q, EcLevel::H] {
            let cap_bits = ec_blocks(v, ec).total_data_codewords() as usize * 8;
            for mode in [Mode::Numeric, Mode::Alphanumeric, Mode::Byte] {
                let cc = mode.char_count_bits(v).unwrap() as usize;
                let max_repr = (1usize << cc) - 1;
                // Max chars that fit: invert payload_bits.
                let payload = cap_bits.saturating_sub(4 + cc);
                let max_chars = match mode {
                    Mode::Numeric => payload * 3 / 10,
                    Mode::Alphanumeric => payload * 2 / 11,
                    Mode::Byte => payload / 8,
                    _ => 0,
                };
                assert!(
                    max_chars <= max_repr,
                    "v{n} {ec:?} {mode:?}: capacity admits {max_chars} chars but indicator holds {max_repr}"
                );
            }
        }
    }
}

#[test]
fn exact_capacity_fits_and_roundtrips() {
    // Byte mode, payload sized to exactly fill the version's data capacity.
    for n in [1u8, 2, 7, 20, 40] {
        let v = Version::new(n).unwrap();
        for ec in [EcLevel::L, EcLevel::H] {
            let cap = ec_blocks(v, ec).total_data_codewords() as usize;
            let cc = Mode::Byte.char_count_bits(v).unwrap() as usize;
            // exact fit: 4 + cc + 8*len == cap*8  =>  len = cap - ceil((4+cc)/8)
            let len = cap - (4 + cc).div_ceil(8);
            let payload: Vec<u8> = (0..len).map(|i| (i as u8) ^ 0x7E).collect();
            let opts = Options {
                ec,
                version: None,
                mask: None,
                mode: Some(Mode::Byte),
                eci: None,
                assume_shift_jis: false,
                single_segment: false,
                boost_ec: false,
            };
            let m = encode(&payload, opts).unwrap_or_else(|e| panic!("v{n} {ec:?} exact: {e:?}"));
            let d = qr_decode::decode(&m).unwrap_or_else(|e| panic!("v{n} {ec:?} decode: {e:?}"));
            assert_eq!(d.bytes, payload, "v{n} {ec:?}");
            // Auto version selection must not overshoot past the exact fit.
            assert!(
                d.version.number() <= n,
                "v{n} {ec:?}: overshot to v{}",
                d.version.number()
            );
        }
    }
}

#[test]
fn one_over_capacity_is_clean_error() {
    for n in [1u8, 5, 40] {
        let v = Version::new(n).unwrap();
        let cap = ec_blocks(v, EcLevel::H).total_data_codewords() as usize;
        let cc = Mode::Byte.char_count_bits(v).unwrap() as usize;
        let len = cap - (4 + cc).div_ceil(8) + 1;
        let payload = vec![0u8; len];
        let opts = Options {
            ec: EcLevel::H,
            version: Some(v),
            mask: None,
            mode: Some(Mode::Byte),
            eci: None,
            assume_shift_jis: false,
            single_segment: false,
            boost_ec: false,
        };
        assert!(
            encode(&payload, opts).is_err(),
            "v{n} over-capacity must error"
        );
    }
}

#[test]
fn forced_mode_incompatible_errors_not_panics() {
    let opts = |m| Options {
        ec: EcLevel::M,
        version: None,
        mask: None,
        mode: Some(m),
        eci: None,
        assume_shift_jis: false,
        single_segment: false,
        boost_ec: false,
    };
    assert!(encode(b"abc", opts(Mode::Numeric)).is_err());
    assert!(encode(b"abc", opts(Mode::Alphanumeric)).is_err());
    assert!(encode(b"\xff\xfe", opts(Mode::Kanji)).is_err());
    assert!(encode(b"123", opts(Mode::Eci)).is_err());
}

#[test]
fn invalid_mask_errors() {
    let opts = Options {
        ec: EcLevel::M,
        version: None,
        mask: Some(8),
        mode: None,
        eci: None,
        assume_shift_jis: false,
        single_segment: false,
        boost_ec: false,
    };
    assert!(encode(b"123", opts).is_err());
}
