//! Suite 5 — seeded pseudo-fuzz over the matrix roundtrip.
//!
//! A deterministic xorshift64* stream picks the version, EC level, content kind
//! and payload length, then the payload bytes themselves. Everything is
//! reproducible from the seed, so a failure prints the exact case and re-runs
//! identically.

mod common;

use common::{ALL_CONTENT, ALL_EC, Content, Rng, ec_label, max_chars, payload, ver};
use qr_decode::decode;
use qr_encode::{Options, encode};

/// One fuzz case: encode a random payload into a random version/EC and require
/// a byte-exact decode.
fn fuzz_case(rng: &mut Rng, iteration: usize) {
    let version = ver(1 + rng.below(40) as u8);
    let ec = ALL_EC[rng.below(ALL_EC.len())];
    let kind = ALL_CONTENT[rng.below(ALL_CONTENT.len())];

    let cap = max_chars(kind.mode(), version, ec);
    if cap == 0 {
        return;
    }
    let len = 1 + rng.below(cap);

    // Fully random bytes for the binary kind; structured payloads otherwise so
    // the encoder's auto mode selection is exercised across all three modes.
    let data = if kind == Content::Binary {
        (0..len).map(|_| (rng.next_u32() & 0xFF) as u8).collect()
    } else {
        payload(kind, len)
    };

    let mut opts = Options::new(ec);
    opts.version = Some(version);
    let m = encode(&data, opts).unwrap_or_else(|e| {
        panic!(
            "iter {iteration}: encode failed for v{}-{} {} len {len}: {e}",
            version.number(),
            ec_label(ec),
            kind.label()
        )
    });
    let d = decode(&m).unwrap_or_else(|e| {
        panic!(
            "iter {iteration}: decode failed for v{}-{} {} len {len}: {e}",
            version.number(),
            ec_label(ec),
            kind.label()
        )
    });
    assert_eq!(
        d.bytes,
        data,
        "iter {iteration}: payload mismatch for v{}-{} {} len {len}",
        version.number(),
        ec_label(ec),
        kind.label()
    );
    assert_eq!(d.version.number(), version.number(), "iter {iteration}: version");
    assert_eq!(d.ec, ec, "iter {iteration}: ec");
    assert_eq!(d.errors_corrected, 0, "iter {iteration}: clean symbol");
}

/// Runs `count` cases from `seed`, offsetting the iteration labels by `base`.
fn run(seed: u64, base: usize, count: usize) {
    let mut rng = Rng::new(seed);
    for i in 0..count {
        fuzz_case(&mut rng, base + i);
    }
}

#[test]
fn fuzz_roundtrip_seed_a() {
    run(0x5EED_0001, 0, 50);
}

#[test]
fn fuzz_roundtrip_seed_b() {
    run(0xC0FF_EE42, 50, 50);
}

#[test]
fn fuzz_roundtrip_seed_c() {
    run(0x1234_5678_9ABC_DEF0, 100, 50);
}

#[test]
fn fuzz_roundtrip_seed_d() {
    run(0xDEAD_BEEF_CAFE_BABE, 150, 50);
}

/// Empty and single-byte payloads at auto-selected versions — the degenerate
/// end of the input space, kept deterministic and separate from the sweep.
#[test]
fn fuzz_degenerate_payloads() {
    for ec in ALL_EC {
        let m = encode(b"", Options::new(ec)).expect("encode empty payload");
        let d = decode(&m).expect("decode empty payload");
        assert!(d.bytes.is_empty(), "empty payload must decode to no bytes");

        for byte in [0x00u8, 0x0A, 0x7F, 0xFF] {
            let m = encode(&[byte], Options::new(ec)).expect("encode single byte");
            let d = decode(&m).expect("decode single byte");
            assert_eq!(d.bytes, vec![byte], "single byte {byte:#04x} at {}", ec_label(ec));
        }
    }
}
