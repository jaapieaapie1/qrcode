//! Suite 6 — multiscan: several distinct QR codes in a single image, all
//! located and decoded by one `qr_detect::detect` call.

mod common;

use common::{Gray, add_noise, render_gray, rotate, ver};
use qr_core::EcLevel;
use qr_encode::{Options, encode};

/// Composites `code` onto an existing canvas (unlike `common::compose`, which
/// starts a fresh one) with its top-left corner at `(x, y)`.
fn blit(canvas: &mut Gray, code: &Gray, x: u32, y: u32) {
    for sy in 0..code.h {
        for sx in 0..code.w {
            canvas.set(x + sx, y + sy, code.get(sx, sy));
        }
    }
}

fn encode_gray(payload: &[u8], version: u8, ec: EcLevel, scale: u32) -> Gray {
    let mut opts = Options::new(ec);
    opts.version = Some(ver(version));
    let m = encode(payload, opts).expect("encode multiscan fixture");
    render_gray(&m, scale, 4)
}

/// Collects the decoded payloads of every detection, sorted for comparison.
fn decoded_payloads(img: &Gray) -> Vec<Vec<u8>> {
    let mut got: Vec<Vec<u8>> = qr_detect::detect(&img.to_image())
        .into_iter()
        .filter_map(|d| d.decoded.map(|dec| dec.bytes))
        .collect();
    got.sort();
    got
}

/// Three codes of different versions, EC levels, and payloads in one image —
/// all three must come back decoded, byte-exact, with no phantom extras.
#[test]
fn three_codes_one_image() {
    let a = encode_gray(b"ALPHA-1", 1, EcLevel::M, 8);
    let b = encode_gray(b"code two, longer payload with bytes", 4, EcLevel::Q, 8);
    let c = encode_gray(b"3333333333", 7, EcLevel::L, 8);

    let mut canvas = Gray::new(900, 760, 255);
    blit(&mut canvas, &a, 30, 40);
    blit(&mut canvas, &b, 450, 40);
    blit(&mut canvas, &c, 20, 320);

    let dets = qr_detect::detect(&canvas.to_image());
    assert_eq!(dets.len(), 3, "exactly three candidates for three symbols");

    let mut want: Vec<Vec<u8>> = vec![
        b"ALPHA-1".to_vec(),
        b"code two, longer payload with bytes".to_vec(),
        b"3333333333".to_vec(),
    ];
    want.sort();
    assert_eq!(decoded_payloads(&canvas), want, "all three payloads exact");
}

/// Two codes close together (separated by roughly one quiet zone) must not be
/// merged or shadowed by each other's finder patterns.
#[test]
fn two_adjacent_codes() {
    let a = encode_gray(b"LEFT", 2, EcLevel::M, 8);
    let b = encode_gray(b"RIGHT", 2, EcLevel::M, 8);
    let gap = 16; // ~2 modules beyond the rendered quiet zones

    let mut canvas = Gray::new(a.w + b.w + gap + 40, a.h + 40, 255);
    blit(&mut canvas, &a, 20, 20);
    blit(&mut canvas, &b, 20 + a.w + gap, 20);

    let want = vec![b"LEFT".to_vec(), b"RIGHT".to_vec()];
    assert_eq!(
        decoded_payloads(&canvas),
        want,
        "adjacent pair both decoded"
    );
}

/// One upright and one rotated code in the same image, plus mild sensor noise:
/// both decode.
#[test]
fn mixed_orientation_with_noise() {
    let upright = encode_gray(b"UPRIGHT", 3, EcLevel::M, 8);
    let tilted = rotate(
        &encode_gray(b"TILTED", 3, EcLevel::M, 8),
        25f64.to_radians(),
    );

    let mut canvas = Gray::new(upright.w + tilted.w + 80, tilted.h + 60, 255);
    blit(&mut canvas, &upright, 20, 30);
    blit(&mut canvas, &tilted, upright.w + 50, 10);
    add_noise(&mut canvas, 12, 0x5eed);

    let mut want = vec![b"UPRIGHT".to_vec(), b"TILTED".to_vec()];
    want.sort();
    assert_eq!(
        decoded_payloads(&canvas),
        want,
        "mixed orientation pair decoded"
    );
}
