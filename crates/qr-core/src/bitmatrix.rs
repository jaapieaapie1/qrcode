//! A square, bit-packed module grid shared across every crate in the stack.

use alloc::vec;
use alloc::vec::Vec;

/// A square grid of one-bit modules.
///
/// Storage is bit-packed row-major (`size * size` bits rounded up to whole
/// bytes). Coordinates are `(x, y)` where `x` is the column and `y` is the row,
/// with the origin at the top-left corner. A set bit (`true`) is a dark module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitMatrix {
    size: u32,
    bits: Vec<u8>,
}

impl BitMatrix {
    /// Creates an all-light (`false`) matrix of the given side length.
    pub fn new(size: u32) -> BitMatrix {
        let cells = (size as usize) * (size as usize);
        let bytes = cells.div_ceil(8);
        BitMatrix {
            size,
            bits: vec![0u8; bytes],
        }
    }

    /// The side length of the grid.
    #[inline]
    pub fn size(self: &BitMatrix) -> u32 {
        self.size
    }

    #[inline]
    fn index(&self, x: u32, y: u32) -> Option<(usize, u8)> {
        if x >= self.size || y >= self.size {
            return None;
        }
        let bit = (y as usize) * (self.size as usize) + (x as usize);
        Some((bit / 8, (bit % 8) as u8))
    }

    /// Returns the module at `(x, y)` (column `x`, row `y`).
    ///
    /// Out-of-bounds coordinates read as `false` rather than panicking.
    #[inline]
    pub fn get(&self, x: u32, y: u32) -> bool {
        match self.index(x, y) {
            Some((byte, bit)) => (self.bits[byte] >> bit) & 1 == 1,
            None => false,
        }
    }

    /// Sets the module at `(x, y)` (column `x`, row `y`).
    ///
    /// Out-of-bounds coordinates are ignored.
    #[inline]
    pub fn set(&mut self, x: u32, y: u32, value: bool) {
        if let Some((byte, bit)) = self.index(x, y) {
            if value {
                self.bits[byte] |= 1 << bit;
            } else {
                self.bits[byte] &= !(1 << bit);
            }
        }
    }

    /// Flips the module at `(x, y)` and returns its new value.
    ///
    /// Out-of-bounds coordinates are ignored and return `false`.
    #[inline]
    pub fn toggle(&mut self, x: u32, y: u32) -> bool {
        let v = !self.get(x, y);
        self.set(x, y, v);
        v
    }
}
