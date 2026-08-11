//! qr-encode: full `encode()` at three sizes, plus the Micro QR encoder.
//!
//! Two stages inside `encode()` are worth attributing separately: the
//! dynamic-programming segmenter, and mask selection (eight candidate matrices,
//! each fully placed and penalty-scored). `Options` exposes a lever for each, so
//! both are measured rather than inferred:
//!
//! * DP — same payload, same forced version, `mode: None` vs
//!   `mode: Some(Mode::Byte)`. The gap is the segmenter.
//! * mask selection — same payload, `mask: None` (all eight scored) vs
//!   `mask: Some(0)` (one placed). The gap is the selection stage.

mod common;

use benchmarks::{Unit, bench_units, group};
use common::{must_encode, pseudo_bytes};
use qr_core::{EcLevel, Mode, Version};
use qr_encode::Options;
use std::hint::black_box;

/// v1 numeric payload: the ISO worked example's eight digits.
const V1_NUMERIC: &[u8] = b"01234567";

/// Largest byte payload a v40-L symbol holds: 2956 data codewords minus the
/// 4-bit mode indicator and 16-bit character count.
const V40_BYTE_LEN: usize = 2953;

/// A URL-shaped payload that no single mode encodes well: an uppercase
/// alphanumeric host and path, lowercase byte-mode fragments, and long digit
/// runs. Exactly the input the segmentation DP exists for.
fn url_payload(repeats: usize, tail: usize) -> Vec<u8> {
    let mut s = String::from("HTTPS://EXAMPLE.COM/TRACK?");
    for i in 0..repeats {
        s.push_str("id=");
        s.push_str(&format!("{:09}", i * 7919));
        s.push_str("&ref=Order-2026-08-11-");
        s.push_str(&format!("{i:03}"));
        s.push_str("&SKU=AB-");
        s.push_str(&format!("{:06}", i * 31));
        s.push('&');
    }
    s.push_str("note=");
    s.extend((0..tail).map(|i| char::from(b'a' + (i % 26) as u8)));
    s.into_bytes()
}

/// The shortest [`url_payload`] tail that makes the payload land on `target`.
///
/// Searched rather than hard-coded so the fixture stays on the version it
/// claims even if a capacity table is ever corrected — a bench that silently
/// slid to v9 or v11 would be worse than one that fails loudly.
fn url_payload_for_version(target: u8, ec: EcLevel) -> Vec<u8> {
    let want = 17 + 4 * u32::from(target);
    for tail in 0..1024 {
        let payload = url_payload(3, tail);
        match must_encode(&payload, Options::new(ec)).size() {
            size if size == want => return payload,
            size if size > want => break,
            _ => {}
        }
    }
    panic!("no URL payload lands on version {target} at {ec:?}");
}

fn v(n: u8) -> Version {
    Version::new(n).expect("valid version")
}

fn main() {
    let v10_payload = url_payload_for_version(10, EcLevel::M);
    let v40_payload = pseudo_bytes(V40_BYTE_LEN, 0xdead_beef);

    // Fixture sanity: assert the versions these benches claim to measure, so a
    // capacity-table change can never silently move a bench to another size.
    assert_eq!(
        must_encode(V1_NUMERIC, Options::new(EcLevel::M)).size(),
        21,
        "v1 numeric fixture must land on version 1"
    );
    assert_eq!(
        must_encode(&v10_payload, Options::new(EcLevel::M)).size(),
        57,
        "URL fixture must land on version 10 at level M"
    );
    assert_eq!(
        must_encode(&v40_payload, Options::new(EcLevel::L)).size(),
        177,
        "max byte fixture must land on version 40 at level L"
    );

    group("qr-encode — encode()");

    bench_units(
        "encode v1-M, 8 numeric digits",
        20_000,
        Unit::Bytes(V1_NUMERIC.len() as u64),
        || must_encode(black_box(V1_NUMERIC), Options::new(EcLevel::M)),
    );

    bench_units(
        &format!(
            "encode v10-M, {}B URL-ish (auto version)",
            v10_payload.len()
        ),
        20_000,
        Unit::Bytes(v10_payload.len() as u64),
        || must_encode(black_box(&v10_payload), Options::new(EcLevel::M)),
    );

    bench_units(
        &format!("encode v40-L, {V40_BYTE_LEN}B byte (auto version)"),
        2_000,
        Unit::Bytes(V40_BYTE_LEN as u64),
        || must_encode(black_box(&v40_payload), Options::new(EcLevel::L)),
    );

    group("qr-encode — DP segmentation cost (same version, DP on vs off)");

    for (label, payload, ec, version) in [
        ("v10-M URL-ish", &v10_payload, EcLevel::M, 10u8),
        ("v40-L byte", &v40_payload, EcLevel::L, 40u8),
    ] {
        let pinned = Options {
            version: Some(v(version)),
            ..Options::new(ec)
        };
        let no_dp = Options {
            mode: Some(Mode::Byte),
            ..pinned
        };
        bench_units(
            &format!("{label}, version pinned, DP on"),
            20_000,
            Unit::Bytes(payload.len() as u64),
            || must_encode(black_box(payload), pinned),
        );
        bench_units(
            &format!("{label}, version pinned, mode=Byte (no DP)"),
            20_000,
            Unit::Bytes(payload.len() as u64),
            || must_encode(black_box(payload), no_dp),
        );
    }

    group("qr-encode — mask selection cost (auto vs pinned)");

    // `Options::mask` is the same lever `Options::mode` gave us for the DP:
    // `None` scores all eight candidate masks, `Some(0)` places exactly one. The
    // difference is the entire mask-selection stage, measured rather than
    // inferred from what is left over.
    for (label, payload, ec) in [
        ("encode v1-M, 8 digits", V1_NUMERIC, EcLevel::M),
        ("encode v40-L, 2953B", v40_payload.as_slice(), EcLevel::L),
    ] {
        let auto = Options::new(ec);
        let pinned = Options {
            mask: Some(0),
            ..auto
        };
        bench_units(
            &format!("{label}: mask auto (8 candidates)"),
            20_000,
            Unit::Bytes(payload.len() as u64),
            || must_encode(black_box(payload), auto),
        );
        bench_units(
            &format!("{label}: mask = Some(0) pinned"),
            20_000,
            Unit::Bytes(payload.len() as u64),
            || must_encode(black_box(payload), pinned),
        );
    }

    #[cfg(feature = "micro")]
    {
        use qr_core::micro::{MicroEcLevel, MicroVersion};
        use qr_encode::MicroOptions;

        group("qr-encode — encode_micro()");

        // M4-Q holds 9 alphanumeric characters.
        let payload = b"HELLO M4Q";
        let opts = MicroOptions {
            version: Some(MicroVersion::M4),
            ..MicroOptions::new(MicroEcLevel::Q)
        };
        assert_eq!(
            qr_encode::encode_micro(payload, opts)
                .expect("M4-Q fixture must encode")
                .size(),
            17
        );

        bench_units(
            "encode_micro M4-Q, 9 alphanumeric chars",
            50_000,
            Unit::Bytes(payload.len() as u64),
            || qr_encode::encode_micro(black_box(payload), opts).expect("M4-Q fixture"),
        );
    }

    println!();
}
