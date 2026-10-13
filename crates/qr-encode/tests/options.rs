//! `Options::single_segment` and `Options::boost_ec`, checked through the
//! independent decoder.

use qr_core::{BitMatrix, EcLevel, Mode, Version};
use qr_encode::{EncodeError, Options, encode};

fn decode(m: &BitMatrix) -> qr_decode::Decoded {
    qr_decode::decode(m).expect("decode")
}

fn single(ec: EcLevel) -> Options {
    Options {
        single_segment: true,
        ..Options::new(ec)
    }
}

fn forced(ec: EcLevel, mode: Mode) -> Options {
    Options {
        mode: Some(mode),
        ..Options::new(ec)
    }
}

fn boosted(ec: EcLevel) -> Options {
    Options {
        boost_ec: true,
        ..Options::new(ec)
    }
}

// ---- single_segment --------------------------------------------------------

#[test]
fn single_segment_picks_the_narrowest_mode_holding_everything() {
    let cases: [(&[u8], Mode); 5] = [
        (b"0123456789", Mode::Numeric),
        (b"", Mode::Numeric),
        (b"HELLO WORLD 123", Mode::Alphanumeric),
        (b"Hello world 123", Mode::Byte),
        (b"\xFF\x00binary", Mode::Byte),
    ];
    for (data, mode) in cases {
        let got = encode(data, single(EcLevel::M)).unwrap();
        assert_eq!(
            got,
            encode(data, forced(EcLevel::M, mode)).unwrap(),
            "{data:?}"
        );
        assert_eq!(decode(&got).bytes, data);
    }
}

#[test]
fn single_segment_differs_from_mixed_segmentation() {
    // One lowercase letter drags 40 digits into byte mode: version 3 at M
    // instead of the mixed byte + numeric version 2.
    let data = b"a0123456789012345678901234567890123456789";
    let mixed = encode(data, Options::new(EcLevel::M)).unwrap();
    let one = encode(data, single(EcLevel::M)).unwrap();
    assert!(one.size() > mixed.size());
    assert_eq!(one, encode(data, forced(EcLevel::M, Mode::Byte)).unwrap());
    assert_eq!(decode(&one).bytes, data);
}

#[test]
fn single_segment_uses_kanji_only_when_asserted() {
    // Shift-JIS for "点茗" (ISO 7.4.6 examples).
    let data = [0x93, 0x5F, 0xE4, 0xAA];
    let plain = encode(&data, single(EcLevel::M)).unwrap();
    assert_eq!(
        plain,
        encode(&data, forced(EcLevel::M, Mode::Byte)).unwrap()
    );
    let sjis = Options {
        assume_shift_jis: true,
        ..single(EcLevel::M)
    };
    let kanji = encode(&data, sjis).unwrap();
    assert_eq!(
        kanji,
        encode(&data, forced(EcLevel::M, Mode::Kanji)).unwrap()
    );
    assert_eq!(decode(&kanji).bytes, data);
}

#[test]
fn forced_mode_wins_over_single_segment() {
    let opts = Options {
        mode: Some(Mode::Byte),
        ..single(EcLevel::L)
    };
    assert_eq!(
        encode(b"12345", opts).unwrap(),
        encode(b"12345", forced(EcLevel::L, Mode::Byte)).unwrap()
    );
}

// ---- boost_ec ----------------------------------------------------------------

#[test]
fn boost_raises_the_level_within_the_version() {
    // 3 alphanumeric characters fit version 1 even at H.
    let d = decode(&encode(b"MAG", boosted(EcLevel::Q)).unwrap());
    assert_eq!((d.version.number(), d.ec), (1, EcLevel::H));
    // 11 alphanumeric characters: version 1 holds 16 at Q, 10 at H.
    let d = decode(
        &encode(
            b"ECC LEVEL L",
            Options {
                boost_ec: true,
                ..single(EcLevel::L)
            },
        )
        .unwrap(),
    );
    assert_eq!((d.version.number(), d.ec), (1, EcLevel::Q));
}

#[test]
fn boost_off_keeps_the_requested_level() {
    let d = decode(&encode(b"MAG", Options::new(EcLevel::L)).unwrap());
    assert_eq!(d.ec, EcLevel::L);
}

#[test]
fn boost_never_grows_the_symbol_and_stops_at_the_last_fitting_level() {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..300 {
        let len = (next() % 400) as usize;
        let data: Vec<u8> = (0..len).map(|_| next() as u8).collect();
        for ec in EcLevel::ALL {
            let plain = encode(&data, Options::new(ec)).unwrap();
            let up = decode(&encode(&data, boosted(ec)).unwrap());
            assert_eq!(up.version.size(), plain.size());
            assert!(up.ec as u8 >= ec as u8);
            assert_eq!(up.bytes, data);
            // The next level up does not hold the data in this version.
            let higher = EcLevel::ALL
                .into_iter()
                .find(|&l| l as u8 == up.ec as u8 + 1);
            if let Some(higher) = higher {
                let pinned = Options {
                    version: Some(up.version),
                    ..Options::new(higher)
                };
                assert_eq!(encode(&data, pinned), Err(EncodeError::DataTooLarge));
            }
        }
    }
}

#[test]
fn boost_applies_to_a_forced_version() {
    let opts = Options {
        version: Some(Version::new(5).unwrap()),
        ..boosted(EcLevel::L)
    };
    let d = decode(&encode(b"HELLO", opts).unwrap());
    assert_eq!((d.version.number(), d.ec), (5, EcLevel::H));
}
