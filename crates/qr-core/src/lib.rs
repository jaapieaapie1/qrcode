//! `qr-core` — the dependency-free, `no_std` foundation for the QR stack.
//!
//! This crate hosts the primitives that every other crate builds on:
//! [`Version`]/[`EcLevel`]/[`Mode`] metadata, the [`BitMatrix`] module grid,
//! GF(256) arithmetic ([`gf`]), Reed-Solomon coding ([`reed_solomon`]), BCH
//! format/version information ([`bch`]), the error-correction block tables
//! ([`ec_blocks`]), the eight data masks ([`mask_bit`]) and the function-pattern
//! map ([`is_function_module`]).
//!
//! Everything is hand-rolled with no external dependencies. `alloc` is used for
//! the dynamically sized [`BitMatrix`] and the Reed-Solomon working buffers.
//!
//! # Coordinates
//!
//! All public APIs that take coordinates use `(x, y)` = (column, row) with the
//! origin at the top-left corner.
//!
//! # Error handling
//!
//! Library paths never panic on malformed input; they return [`Result`] or
//! [`Option`].

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

#[cfg(test)]
extern crate std;

pub mod bch;
pub mod gf;
pub mod mask;
pub mod reed_solomon;

mod bitmatrix;
mod blocks;
mod ec;
mod function;
mod mode;
mod version;

pub use bitmatrix::BitMatrix;
pub use blocks::{EcBlocks, ec_blocks};
pub use ec::EcLevel;
pub use function::is_function_module;
pub use mask::mask_bit;
pub use mode::Mode;
pub use version::Version;

/// Errors returned by the core primitives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// A version number outside `1..=40`.
    InvalidVersion,
    /// A buffer whose length is incompatible with the requested operation.
    InvalidLength,
    /// A Reed-Solomon block with more errors than the code can correct.
    TooManyErrors,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let msg = match self {
            Error::InvalidVersion => "version out of range (1..=40)",
            Error::InvalidLength => "invalid buffer length",
            Error::TooManyErrors => "too many errors to correct",
        };
        f.write_str(msg)
    }
}

impl core::error::Error for Error {}
