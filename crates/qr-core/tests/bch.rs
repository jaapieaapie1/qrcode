//! Format- and version-information BCH roundtrip and bit-error correction.

use qr_core::bch::{
    format_info_decode, format_info_encode, version_info_decode, version_info_encode,
};
use qr_core::EcLevel;

#[test]
fn format_info_roundtrip_all() {
    for ec in EcLevel::ALL {
        for mask in 0..8u8 {
            let word = format_info_encode(ec, mask);
            assert_eq!(word & !0x7FFF, 0, "format word must be 15 bits");
            assert_eq!(format_info_decode(word), Some((ec, mask)));
        }
    }
}

#[test]
fn format_info_all_words_distinct() {
    let mut words = Vec::new();
    for ec in EcLevel::ALL {
        for mask in 0..8u8 {
            words.push(format_info_encode(ec, mask));
        }
    }
    // The 32 valid words form a BCH code with minimum distance 7; all distinct.
    for i in 0..words.len() {
        for j in (i + 1)..words.len() {
            let dist = (words[i] ^ words[j]).count_ones();
            assert!(dist >= 7, "format words {i},{j} distance {dist} < 7");
        }
    }
}

#[test]
fn format_info_corrects_up_to_3_bitflips() {
    for ec in EcLevel::ALL {
        for mask in 0..8u8 {
            let word = format_info_encode(ec, mask);
            for flips in 1..=3u32 {
                // Flip `flips` spread-out bits.
                let mut corrupted = word;
                for k in 0..flips {
                    corrupted ^= 1 << (k * 5);
                }
                assert_eq!(
                    format_info_decode(corrupted),
                    Some((ec, mask)),
                    "ec {ec:?} mask {mask} with {flips} flips"
                );
            }
        }
    }
}

#[test]
fn version_info_roundtrip_all() {
    for v in 7..=40u8 {
        let word = version_info_encode(v).unwrap();
        assert_eq!(word & !0x3FFFF, 0, "version word must be 18 bits");
        assert_eq!(version_info_decode(word), Some(v));
    }
}

#[test]
fn version_info_none_below_7() {
    assert!(version_info_encode(6).is_none());
    assert!(version_info_encode(1).is_none());
    assert!(version_info_encode(41).is_none());
}

#[test]
fn version_info_corrects_up_to_3_bitflips() {
    for v in 7..=40u8 {
        let word = version_info_encode(v).unwrap();
        for flips in 1..=3u32 {
            let mut corrupted = word;
            for k in 0..flips {
                corrupted ^= 1 << (k * 6);
            }
            assert_eq!(
                version_info_decode(corrupted),
                Some(v),
                "version {v} with {flips} flips"
            );
        }
    }
}

/// Known format-info value from ISO/IEC 18004: EC level M, mask 101 encodes to
/// the string 100000011001110 = 0x40CE.
#[test]
fn format_info_known_value() {
    assert_eq!(format_info_encode(EcLevel::M, 0b101), 0x40CE);
}
