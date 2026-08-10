//! Passes the linker script to the binary target.
//!
//! These arguments used to live in `[target.*] rustflags` in
//! `.cargo/config.toml`. They do not any more, and the reason is worth keeping:
//! a `RUSTFLAGS` environment variable **replaces** that config table wholesale
//! rather than merging with it, so the workflow-level `RUSTFLAGS: -D warnings`
//! in CI silently removed `-T link.ld` and the link failed on an undefined
//! `__ebss`. `cargo::rustc-link-arg-bins` is additive and nothing in the
//! environment can clobber it, which deletes that whole failure class instead
//! of just today's instance of it.
//!
//! The script is named by absolute path, so the link no longer depends on which
//! directory cargo happens to be standing in either.
//!
//! Target selection stays in `.cargo/config.toml`: it is not a rustflag, and a
//! build script cannot set it.

use std::env;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let script = manifest.join("link.ld");

    println!("cargo::rustc-link-arg-bins=-T{}", script.display());
    // Do not page-align the output sections: the ELF is loaded straight into
    // RAM by QEMU, so the 4 KiB gaps would be pure waste.
    println!("cargo::rustc-link-arg-bins=--nmagic");

    println!("cargo::rerun-if-changed={}", script.display());
    println!("cargo::rerun-if-changed=build.rs");
}
