//! A most-significant-bit-first reader over a byte slice.

/// Reads fixed-width big-endian bit fields out of a codeword byte slice.
///
/// Bits are consumed MSB-first within each byte, matching the QR data
/// bit-stream layout (ISO/IEC 18004 §8.4).
pub(crate) struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    /// Creates a reader positioned at the first bit of `data`.
    pub(crate) fn new(data: &'a [u8]) -> BitReader<'a> {
        BitReader { data, pos: 0 }
    }

    /// Number of bits not yet consumed.
    pub(crate) fn remaining(&self) -> usize {
        self.data.len() * 8 - self.pos
    }

    /// Reads the next `n` bits (`0..=32`) as an unsigned integer.
    ///
    /// Returns `None` if fewer than `n` bits remain or `n > 32`.
    pub(crate) fn read(&mut self, n: u32) -> Option<u32> {
        if n == 0 {
            return Some(0);
        }
        if n > 32 || self.pos + n as usize > self.data.len() * 8 {
            return None;
        }
        let mut value = 0u32;
        for _ in 0..n {
            let byte = self.data[self.pos >> 3];
            let bit = (byte >> (7 - (self.pos & 7))) & 1;
            value = (value << 1) | bit as u32;
            self.pos += 1;
        }
        Some(value)
    }
}
