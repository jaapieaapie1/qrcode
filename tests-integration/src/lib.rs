//! Cross-crate integration tests for the QR workspace.
//!
//! This crate ships no library code: it exists only to host the `tests/`
//! directory, where `qr-core`, `qr-encode`, `qr-decode`, `qr-render`,
//! `qr-detect` and `qr-video` are exercised together. Everything the suites
//! need is a dev-dependency, so nothing here is compiled into a release build.
//!
//! | suite | file |
//! |---|---|
//! | full stack encode -> render -> detect -> decode | `tests/full_roundtrip.rs` |
//! | matrix-level encode -> decode sweep + ISO Annex I | `tests/matrix_roundtrip.rs` |
//! | warped / noisy image detection | `tests/warped.rs` |
//! | multi-frame video session | `tests/video_session.rs` |
//! | seeded pseudo-fuzz | `tests/fuzz_seeded.rs` |
//! | shared helpers | `tests/common/mod.rs` |
