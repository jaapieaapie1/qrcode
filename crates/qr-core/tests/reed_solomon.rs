//! Reed-Solomon known-answer, generator, and corruption-recovery tests.

use qr_core::gf;
use qr_core::reed_solomon::{generator_poly, rs_correct, rs_encode};

/// The 16 version-1-M data codewords for the numeric string "01234567"
/// (ISO/IEC 18004 Annex I worked example).
const V1M_DATA: [u8; 16] = [
    0x10, 0x20, 0x0C, 0x56, 0x61, 0x80, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11,
];
/// Its 10 error-correction codewords (Annex I).
const V1M_EC: [u8; 10] = [0xA5, 0x24, 0xD4, 0xC1, 0xED, 0x36, 0xC7, 0x87, 0x2C, 0x55];

#[test]
fn generator_poly_10_matches_iso_exponents() {
    // Documented QR generator polynomial for 10 EC codewords, in alpha-exponent
    // form (descending degree).
    let expected = [0u8, 251, 67, 46, 61, 118, 70, 64, 94, 32, 45];
    let g = generator_poly(10);
    assert_eq!(g.len(), 11);
    for (i, &coef) in g.iter().enumerate() {
        assert_eq!(gf::log(coef), expected[i], "coefficient {i}");
    }
}

#[test]
fn iso_annex_i_known_vector() {
    let ec = rs_encode(&V1M_DATA, 10);
    assert_eq!(
        ec.as_slice(),
        &V1M_EC,
        "EC codewords do not match ISO Annex I"
    );
}

#[test]
fn correct_no_errors_returns_zero() {
    let mut cw = [V1M_DATA.to_vec(), V1M_EC.to_vec()].concat();
    assert_eq!(rs_correct(&mut cw, 10).unwrap(), 0);
    assert_eq!(&cw[..16], &V1M_DATA);
}

#[test]
fn correct_up_to_capacity() {
    let full = [V1M_DATA.to_vec(), V1M_EC.to_vec()].concat();
    // ec_len = 10 -> can correct up to 5 symbol errors.
    for num in 1..=5usize {
        let mut cw = full.clone();
        // Corrupt `num` distinct positions with non-zero deltas.
        for k in 0..num {
            let pos = k * 4 + 1; // distinct, within len 26 for k<5
            cw[pos] ^= (0x3D + k as u8) | 1;
        }
        let fixed = rs_correct(&mut cw, 10).expect("should correct within capacity");
        assert_eq!(fixed, num, "corrected count for {num} errors");
        assert_eq!(cw, full, "buffer restored for {num} errors");
    }
}

#[test]
fn too_many_errors_is_clean_error() {
    let full = [V1M_DATA.to_vec(), V1M_EC.to_vec()].concat();
    let mut cw = full.clone();
    // 6 errors exceeds the capacity of 5 for ec_len = 10.
    for k in 0..6usize {
        let pos = k * 3 + 2; // distinct, within len 26 for k<6
        cw[pos] ^= 0x7C ^ ((k as u8 * 17) | 1);
    }
    // Must return an error rather than silently producing a wrong result.
    assert!(rs_correct(&mut cw, 10).is_err());
}

#[test]
fn roundtrip_various_lengths() {
    // Encode/corrupt/correct across a range of data + ec sizes.
    for &(dlen, eclen) in &[(19usize, 7usize), (13, 13), (9, 17), (44, 26), (16, 28)] {
        let data: Vec<u8> = (0..dlen).map(|i| ((i * 37 + 5) & 0xFF) as u8).collect();
        let ec = rs_encode(&data, eclen);
        let full = [data.clone(), ec].concat();
        let cap = eclen / 2;
        let mut cw = full.clone();
        // Spread `cap` distinct error positions across the codeword.
        let step = (cw.len() / (cap + 1)).max(1);
        for k in 0..cap {
            let pos = k * step;
            cw[pos] ^= (0x11 + k as u8) | 1;
        }
        let fixed = rs_correct(&mut cw, eclen).expect("within capacity");
        assert_eq!(fixed, cap);
        assert_eq!(cw, full);
    }
}
