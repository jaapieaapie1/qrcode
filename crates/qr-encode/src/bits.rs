//! A minimal MSB-first bit buffer used to assemble the QR data bit stream.

use alloc::vec::Vec;

/// Accumulates bits most-significant-first and packs them into bytes.
///
/// Bits are appended left-to-right; the first bit pushed becomes the most
/// significant bit of the first output byte. Any trailing bits that do not fill
/// a whole byte are zero-padded when [`BitBuffer::into_bytes`] is called.
#[derive(Debug, Default, Clone)]
pub struct BitBuffer {
    bytes: Vec<u8>,
    /// Number of bits currently used in the final byte (`0..=7`). `0` means the
    /// buffer is byte-aligned and a fresh byte must be pushed for the next bit.
    partial: u8,
}

impl BitBuffer {
    /// Creates an empty buffer.
    pub fn new() -> Self {
        BitBuffer {
            bytes: Vec::new(),
            partial: 0,
        }
    }

    /// Number of bits currently stored.
    pub fn len(&self) -> usize {
        if self.partial == 0 {
            self.bytes.len() * 8
        } else {
            (self.bytes.len() - 1) * 8 + self.partial as usize
        }
    }

    /// Appends a single bit.
    pub fn push_bit(&mut self, bit: bool) {
        if self.partial == 0 {
            self.bytes.push(0);
        }
        if bit {
            let idx = self.bytes.len() - 1;
            let shift = 7 - self.partial;
            self.bytes[idx] |= 1 << shift;
        }
        self.partial = (self.partial + 1) % 8;
    }

    /// Appends the low `count` bits of `value`, most-significant of those first.
    ///
    /// `count` must be `<= 32`; higher bits of `value` are ignored.
    pub fn push_bits(&mut self, value: u32, count: u8) {
        let mut i = count;
        while i > 0 {
            i -= 1;
            self.push_bit((value >> i) & 1 == 1);
        }
    }

    /// Consumes the buffer, returning the packed bytes with the final partial
    /// byte zero-padded on the right.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}
