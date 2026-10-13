//! Segmentation round-trips against the independent decoder.
//!
//! Mixed-mode, Kanji and ECI symbols are produced by `qr_encode::encode` and
//! read back with `qr_decode::decode`, so a segment header the encoder writes
//! but the decoder cannot parse (or parses to different bytes) fails here even
//! though the encoder is internally consistent.
//!
//! Added for the full-segmentation step (optimal mixed mode, Kanji, ECI).

use qr_core::{EcLevel, Mode, Version, ec_blocks};
use qr_encode::{EncodeError, Options, encode};

const LEVELS: [EcLevel; 4] = [EcLevel::L, EcLevel::M, EcLevel::Q, EcLevel::H];

/// "点茗" from the ISO/IEC 18004 §7.4.6 example, as Shift-JIS bytes.
const SJIS_EXAMPLE: [u8; 4] = [0x93, 0x5F, 0xE4, 0xAA];

fn auto(data: &[u8], ec: EcLevel) -> Vec<u8> {
    let m = encode(data, Options::new(ec)).expect("encode");
    qr_decode::decode(&m).expect("decode").bytes
}

/// Default options plus the Shift-JIS assertion, i.e. Kanji segmentation on.
fn sjis_opts(ec: EcLevel) -> Options {
    Options {
        assume_shift_jis: true,
        ..Options::new(ec)
    }
}

// ---- mixed-mode ----------------------------------------------------------

#[test]
fn mixed_mode_strings_roundtrip() {
    let cases: [&[u8]; 8] = [
        b"ABC123456789DEF",
        b"ABC123456789012345678901234567890DEF",
        b"http://example.com/1234567890123456789012",
        b"HELLO WORLD 123",
        b"Order #12345 shipped 2024-05-01 to WAREHOUSE 7",
        b"0123456789abcdefABCDEF $%*+-./:",
        b"1",
        b"a1B2c3",
    ];
    for data in cases {
        for ec in LEVELS {
            assert_eq!(
                auto(data, ec),
                data,
                "{ec:?} {}",
                String::from_utf8_lossy(data)
            );
        }
    }
}

/// Random payloads over a mode-mixing alphabet: every symbol must decode back
/// to exactly the bytes that went in, at every EC level.
#[test]
fn random_mixed_payloads_roundtrip_exactly() {
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for trial in 0..200u32 {
        let len = (next() % 120) as usize + 1;
        let payload: Vec<u8> = (0..len)
            .map(|_| {
                let r = next();
                match (trial + (r >> 32) as u32) % 6 {
                    0 | 1 => b'0' + (r % 10) as u8,
                    2 => b'A' + (r % 26) as u8,
                    3 => b'a' + (r % 26) as u8,
                    4 => [0x93u8, 0x5F, 0xE4, 0xAA, 0x88, 0x9F][(r % 6) as usize],
                    _ => (r % 256) as u8,
                }
            })
            .collect();
        let ec = LEVELS[(trial % 4) as usize];
        assert_eq!(auto(&payload, ec), payload, "trial {trial} ({ec:?})");
    }
}

/// Mixed-mode segmentation must not cost more capacity than the single-mode
/// choice it replaced: the auto-selected version can only shrink.
#[test]
fn mixed_mode_never_needs_a_larger_version() {
    let data = b"ABC123456789012345678901234567890DEF";
    let mixed = encode(data, Options::new(EcLevel::M)).expect("encode");
    let forced = encode(
        data,
        Options {
            ec: EcLevel::M,
            version: None,
            mask: None,
            mode: Some(Mode::Alphanumeric),
            eci: None,
            assume_shift_jis: false,
            single_segment: false,
            boost_ec: false,
        },
    )
    .expect("encode");
    assert!(mixed.size() <= forced.size());
    assert_eq!(qr_decode::decode(&mixed).expect("decode").bytes, data);
}

// ---- Kanji ---------------------------------------------------------------

#[test]
fn shift_jis_payload_roundtrips_through_kanji_segments() {
    for repeats in [1usize, 2, 7, 40] {
        let payload: Vec<u8> = SJIS_EXAMPLE
            .iter()
            .copied()
            .cycle()
            .take(4 * repeats)
            .collect();
        for ec in LEVELS {
            let m = encode(&payload, sjis_opts(ec)).expect("encode");
            let decoded = qr_decode::decode(&m).expect("decode").bytes;
            assert_eq!(decoded, payload, "{repeats} x example, {ec:?}");
        }
    }
}

/// Kanji mode is actually being used when opted in, not merely tolerated: 13
/// bits per pair beats byte mode's 16, so the payload fits a version that the
/// byte-mode encoding of the same bytes overflows.
#[test]
fn kanji_beats_byte_mode_on_capacity() {
    let payload: Vec<u8> = SJIS_EXAMPLE.iter().copied().cycle().take(400).collect();
    let v = Version::new(12).unwrap();
    let cap = ec_blocks(v, EcLevel::L).total_data_codewords() as usize * 8;
    // 200 Kanji chars: 4 + 10 + 200 * 13 = 2614 bits. Byte mode would need
    // 4 + 16 + 400 * 8 = 3220.
    assert!((2614..3220).contains(&cap), "v12-L capacity is {cap} bits");
    let opted_in = Options {
        version: Some(v),
        ..sjis_opts(EcLevel::L)
    };
    let m = encode(&payload, opted_in).expect("kanji fits v12-L");
    assert_eq!(qr_decode::decode(&m).expect("decode").bytes, payload);

    // Without the assertion the same bytes stay in byte mode and overflow.
    let default = Options {
        version: Some(v),
        ..Options::new(EcLevel::L)
    };
    assert_eq!(
        encode(&payload, default),
        Err(EncodeError::DataTooLarge),
        "default options must not reach for Kanji"
    );

    // As does byte mode forced explicitly.
    let byte_forced = Options {
        mode: Some(Mode::Byte),
        ..default
    };
    assert_eq!(
        encode(&payload, byte_forced),
        Err(EncodeError::DataTooLarge)
    );
}

// ---- Kanji is opt-in (regression for review B1) ---------------------------

/// UTF-8 CJK text lands inside the Shift-JIS ranges constantly. With default
/// options it must stay in byte mode: a Kanji segment would make a conformant
/// reader render the 13-bit values as Shift-JIS characters — mojibake — even
/// though this workspace's byte-transparent decoder round-trips it fine.
///
/// For a payload with no numeric or alphanumeric run to split out, the default
/// encoding must be *bit-identical* to byte mode forced. That is exact: any
/// Kanji segment changes the bit stream, and therefore the matrix.
#[test]
fn utf8_cjk_never_produces_kanji_segments_by_default() {
    let cases = [
        "世界世界世界世界",
        "日本語のテキストをここに書きます。これは長い文章のテストです。",
        "漢字カタカナひらがな漢字カタカナひらがな漢字カタカナひらがな",
        "こんにちは、世界。",
    ];
    for text in cases {
        let bytes = text.as_bytes();
        for ec in LEVELS {
            let default = encode(bytes, Options::new(ec)).expect("encode");
            let forced_byte = encode(
                bytes,
                Options {
                    mode: Some(Mode::Byte),
                    ..Options::new(ec)
                },
            )
            .expect("encode");
            assert_eq!(
                default, forced_byte,
                "{text} ({ec:?}): default segmentation differs from pure byte mode"
            );
            let decoded = qr_decode::decode(&default).expect("decode");
            assert_eq!(decoded.bytes, bytes, "{text}");
            assert_eq!(decoded.text.as_deref(), Some(text), "{text}");
        }
    }
}

/// The flag, and only the flag, is what switches Kanji on — and the saving it
/// declines by default is real, so the test above is not passing vacuously.
#[test]
fn opt_in_flag_is_what_switches_kanji_on() {
    let bytes = "世界世界世界世界世界".as_bytes();
    let off = encode(bytes, Options::new(EcLevel::L)).expect("encode");
    let on = encode(bytes, sjis_opts(EcLevel::L)).expect("encode");
    assert_ne!(off, on, "the flag must change the segmentation");
    assert_eq!(qr_decode::decode(&off).expect("decode").bytes, bytes);
    assert_eq!(qr_decode::decode(&on).expect("decode").bytes, bytes);

    // 30 bytes: byte mode is 4 + 8 + 240 = 252 bits at v1, Kanji 4 + 8 + 15 x 13
    // = 207. The saving exists; the default path declines it deliberately.
    let v = Version::new(1).unwrap();
    let byte_only = 4 + Mode::Byte.char_count_bits(v).unwrap() as usize + 8 * bytes.len();
    let with_kanji = 4 + Mode::Kanji.char_count_bits(v).unwrap() as usize + 13 * (bytes.len() / 2);
    assert!(
        with_kanji < byte_only,
        "{with_kanji} vs {byte_only}: the saving must exist to be declined"
    );
}

#[test]
fn forced_kanji_mode_roundtrips_and_rejects_non_sjis() {
    let opts = Options {
        ec: EcLevel::M,
        version: None,
        mask: None,
        mode: Some(Mode::Kanji),
        eci: None,
        assume_shift_jis: false,
        single_segment: false,
        boost_ec: false,
    };
    let m = encode(&SJIS_EXAMPLE, opts).expect("encode");
    assert_eq!(qr_decode::decode(&m).expect("decode").bytes, SJIS_EXAMPLE);

    // Odd length and out-of-range pairs are clean errors, not panics.
    assert_eq!(
        encode(&SJIS_EXAMPLE[..3], opts),
        Err(EncodeError::ModeMismatch)
    );
    assert_eq!(encode(b"\xff\xfe", opts), Err(EncodeError::ModeMismatch));
    assert_eq!(encode(b"HELLO", opts), Err(EncodeError::ModeMismatch));
}

// ---- ECI -----------------------------------------------------------------

#[test]
fn eci_26_tagged_utf8_decodes_to_text() {
    let text = "Grüße, 世界 — 123";
    let opts = Options {
        ec: EcLevel::Q,
        version: None,
        mask: None,
        mode: None,
        eci: Some(26),
        assume_shift_jis: false,
        single_segment: false,
        boost_ec: false,
    };
    let m = encode(text.as_bytes(), opts).expect("encode");
    let decoded = qr_decode::decode(&m).expect("decode");
    assert_eq!(decoded.bytes, text.as_bytes());
    assert_eq!(decoded.text.as_deref(), Some(text));
}

/// One assignment number per designator width (1, 2 and 3 codewords): the
/// decoder must consume exactly the header the encoder wrote, otherwise the
/// payload that follows comes back wrong.
#[test]
fn every_eci_designator_width_roundtrips() {
    for eci in [0u32, 3, 26, 127, 128, 899, 16_383, 16_384, 100_000, 999_999] {
        let payload = b"ECI PAYLOAD 42";
        let opts = Options {
            ec: EcLevel::M,
            version: None,
            mask: None,
            mode: None,
            eci: Some(eci),
            assume_shift_jis: false,
            single_segment: false,
            boost_ec: false,
        };
        let m = encode(payload, opts).unwrap_or_else(|e| panic!("eci {eci}: {e:?}"));
        let decoded = qr_decode::decode(&m).unwrap_or_else(|e| panic!("eci {eci} decode: {e:?}"));
        assert_eq!(decoded.bytes, payload, "eci {eci}");
    }
}

#[test]
fn eci_out_of_range_is_a_clean_error() {
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
    assert_eq!(encode(b"123", opts), Err(EncodeError::InvalidEci));
}

/// The ECI header consumes capacity, so a payload that exactly fills a version
/// without it must spill to the next version with it — never silently overflow.
#[test]
fn eci_header_is_counted_against_capacity() {
    let v = Version::new(1).unwrap();
    let cap = ec_blocks(v, EcLevel::H).total_data_codewords() as usize;
    let len = cap - (4 + 8usize).div_ceil(8);
    let payload = vec![b'x'; len];
    let base = Options {
        ec: EcLevel::H,
        version: Some(v),
        mask: None,
        mode: Some(Mode::Byte),
        eci: None,
        assume_shift_jis: false,
        single_segment: false,
        boost_ec: false,
    };
    assert!(encode(&payload, base).is_ok(), "exact fit without ECI");
    let with_eci = Options {
        eci: Some(26),
        assume_shift_jis: false,
        ..base
    };
    assert_eq!(encode(&payload, with_eci), Err(EncodeError::DataTooLarge));
}
