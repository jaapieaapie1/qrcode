//! A most-significant-bit-first reader over a byte slice.

/// Reads fixed-width big-endian bit fields out of a codeword byte slice.
///
/// Bits are consumed MSB-first within each byte, matching the QR data
/// bit-stream layout (ISO/IEC 18004 §8.4).
pub(crate) struct BitReader<'a> {
    data: &'a [u8],
    /// Number of bits of `data` that may be read. Always `<= data.len() * 8`;
    /// micro symbols end on a 4-bit boundary, so the last nibble of the
    /// backing slice can be padding that must stay out of reach.
    bits: usize,
    pos: usize,
}

impl<'a> BitReader<'a> {
    /// Creates a reader positioned at the first bit of `data`, able to read all
    /// of it.
    pub(crate) fn new(data: &'a [u8]) -> BitReader<'a> {
        BitReader {
            data,
            bits: data.len() * 8,
            pos: 0,
        }
    }

    /// Creates a reader limited to the first `bits` bits of `data`, clamped to
    /// the length of the slice.
    #[cfg(feature = "micro")]
    pub(crate) fn with_bit_len(data: &'a [u8], bits: usize) -> BitReader<'a> {
        BitReader {
            data,
            bits: bits.min(data.len() * 8),
            pos: 0,
        }
    }

    /// Number of bits not yet consumed.
    pub(crate) fn remaining(&self) -> usize {
        self.bits - self.pos
    }

    /// Reads the next `n` bits (`0..=32`) as an unsigned integer.
    ///
    /// Returns `None` if fewer than `n` bits remain or `n > 32`.
    pub(crate) fn read(&mut self, n: u32) -> Option<u32> {
        if n == 0 {
            return Some(0);
        }
        if n > 32 || self.pos + n as usize > self.bits {
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
