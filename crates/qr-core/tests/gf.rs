//! GF(256) field-axiom tests.

use qr_core::gf;

#[test]
fn inverse_property() {
    for a in 1u16..=255 {
        let a = a as u8;
        assert_eq!(gf::mul(a, gf::inv(a)), 1, "a * inv(a) != 1 for a={a}");
    }
}

#[test]
fn exp_log_are_inverse() {
    for a in 1u16..=255 {
        let a = a as u8;
        assert_eq!(gf::exp(gf::log(a) as u16), a, "exp(log(a)) != a for a={a}");
    }
}

#[test]
fn mul_div_roundtrip() {
    for a in 0u16..=255 {
        for b in 1u16..=255 {
            let a = a as u8;
            let b = b as u8;
            assert_eq!(gf::div(gf::mul(a, b), b), a, "div(mul(a,b),b) != a");
        }
    }
}

#[test]
fn pow_matches_repeated_mul() {
    for a in 0u16..=255 {
        let a = a as u8;
        let mut acc = 1u8;
        for n in 0..=10u32 {
            assert_eq!(gf::pow(a, n), acc, "pow({a},{n})");
            acc = gf::mul(acc, a);
        }
    }
}

#[test]
fn zero_and_identity() {
    assert_eq!(gf::add(0xAB, 0xAB), 0);
    assert_eq!(gf::mul(0, 0x5A), 0);
    assert_eq!(gf::mul(1, 0x5A), 0x5A);
    assert_eq!(gf::exp(0), 1);
}
