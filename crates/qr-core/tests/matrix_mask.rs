//! BitMatrix storage and mask-predicate sanity checks.

use qr_core::{BitMatrix, mask_bit};

#[test]
fn bitmatrix_get_set() {
    let mut m = BitMatrix::new(21);
    assert_eq!(m.size(), 21);
    assert!(!m.get(3, 4));
    m.set(3, 4, true);
    assert!(m.get(3, 4));
    assert!(!m.get(4, 3)); // coordinate order (x, y) respected
    m.set(3, 4, false);
    assert!(!m.get(3, 4));
    // Out of bounds is graceful.
    assert!(!m.get(21, 0));
    m.set(100, 100, true); // ignored, no panic
}

#[test]
fn bitmatrix_toggle() {
    let mut m = BitMatrix::new(5);
    assert!(m.toggle(2, 2));
    assert!(m.get(2, 2));
    assert!(!m.toggle(2, 2));
    assert!(!m.get(2, 2));
}

#[test]
fn masks_are_stable_and_in_range() {
    // Each mask must produce both true and false over a small grid and never
    // panic; ids outside 0..8 are always false.
    for id in 0..8u8 {
        let mut seen_true = false;
        let mut seen_false = false;
        for y in 0..7 {
            for x in 0..7 {
                if mask_bit(id, x, y) {
                    seen_true = true;
                } else {
                    seen_false = true;
                }
            }
        }
        assert!(seen_true && seen_false, "mask {id} degenerate");
    }
    assert!(!mask_bit(8, 0, 0));
    assert!(!mask_bit(255, 3, 5));
}

#[test]
fn mask0_checkerboard() {
    // (i + j) % 2 == 0
    assert!(mask_bit(0, 0, 0));
    assert!(!mask_bit(0, 1, 0));
    assert!(!mask_bit(0, 0, 1));
    assert!(mask_bit(0, 1, 1));
}
