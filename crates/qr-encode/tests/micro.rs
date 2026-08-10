//! Micro QR encoder tests.
//!
//! The whole file is gated on the `micro` feature, so a
//! `--no-default-features` run compiles it to zero tests rather than failing to
//! build.
//!
//! Ground truth is ISO/IEC 18004:2024: the Annex I.3 worked example and its
//! printed Figure I.4 symbol, the Table 7 data capacities, the Table 13 symbol
//! numbers and the 7.8.3.2 mask evaluation score. Where a value is derived
//! rather than printed (the M1 and M3 bit streams, which the standard gives no
//! worked example for) the derivation is written out in the comments so the
//! expectation can be checked by hand against the clause it comes from.

#![cfg(feature = "micro")]

use qr_core::BitMatrix;
use qr_core::micro::{
    MicroEcLevel, MicroSymbolInfo, MicroVersion, micro_data_modules, micro_format_info_decode,
    micro_format_info_read, micro_is_function_module, micro_mask_bit, micro_symbols,
};
use qr_core::reed_solomon::rs_encode;
use qr_encode::{MicroEncodeError, MicroOptions, encode_micro};

// -- helpers -----------------------------------------------------------------

fn options(ec: MicroEcLevel, version: Option<MicroVersion>, mask: Option<u8>) -> MicroOptions {
    MicroOptions { ec, version, mask }
}

/// Reads the symbol number and mask back out of the format information.
fn read_format(matrix: &BitMatrix) -> (MicroSymbolInfo, u8) {
    micro_format_info_decode(micro_format_info_read(matrix)).expect("format information decodes")
}

/// Walks the shared placement order and returns the unmasked module bit stream.
fn extract_bits(matrix: &BitMatrix, version: MicroVersion, mask: u8) -> Vec<bool> {
    micro_data_modules(version)
        .map(|(x, y)| matrix.get(x, y) ^ micro_mask_bit(mask, x, y))
        .collect()
}

/// Splits an unmasked module bit stream back into codewords, honouring the 4-bit
/// final data codeword of M1 and M3. That codeword is returned as a value in
/// `0..=15` — zero-prefixed, not zero-suffixed — which is the workspace-wide
/// convention for feeding it to Reed-Solomon (ISO 7.4.11, Annex N.1 precedent),
/// agreed with the qr-decode side.
fn to_codewords(bits: &[bool], info: &MicroSymbolInfo) -> (Vec<u8>, Vec<u8>) {
    let mut cursor = 0usize;
    let mut take = |count: usize| -> u8 {
        let mut value = 0u8;
        for _ in 0..count {
            value = (value << 1) | u8::from(bits[cursor]);
            cursor += 1;
        }
        value
    };

    let mut data = Vec::new();
    for i in 0..info.data_codewords {
        let is_half = info.has_half_final_data_codeword() && i + 1 == info.data_codewords;
        data.push(take(if is_half { 4 } else { 8 }));
    }
    let ec = (0..info.ec_codewords).map(|_| take(8)).collect();
    (data, ec)
}

/// The ISO 7.8.3.2 evaluation score, transcribed from the clause independently
/// of the encoder: `S1` dark modules down the right edge and `S2` along the
/// lower edge, each excluding the final module of the timing pattern on that
/// edge; `Es = min * 16 + max`; highest wins.
fn evaluation_score(matrix: &BitMatrix) -> u32 {
    let size = matrix.size();
    let mut s1 = 0u32;
    let mut s2 = 0u32;
    for i in 1..size {
        if matrix.get(size - 1, i) {
            s1 += 1;
        }
        if matrix.get(i, size - 1) {
            s2 += 1;
        }
    }
    if s1 <= s2 { s1 * 16 + s2 } else { s2 * 16 + s1 }
}

/// Re-masks an encoded symbol with a different mask, leaving the function
/// patterns and the format information alone. Used only to score the masks the
/// encoder did not pick.
fn remask(matrix: &BitMatrix, version: MicroVersion, from: u8, to: u8) -> BitMatrix {
    let mut out = matrix.clone();
    for (x, y) in micro_data_modules(version) {
        let unmasked = matrix.get(x, y) ^ micro_mask_bit(from, x, y);
        out.set(x, y, unmasked ^ micro_mask_bit(to, x, y));
    }
    out
}

// -- ISO Annex I.3 / Figure I.4 ---------------------------------------------

/// The final M2-L symbol for "01234567" as printed in ISO Figure I.4, page 93.
/// Same fixture the qr-core micro tests pin the primitives against; here it pins
/// the whole encoder, end to end, against printed ISO output.
const FIGURE_I4: [&str; 13] = [
    "#######.#.#.#",
    "#.....#.###.#",
    "#.###.#..##.#",
    "#.###.#..####",
    "#.###.#.###..",
    "#.....#.#...#",
    "#######..####",
    ".........##..",
    "##.#....#...#",
    ".##.#.#.#.#.#",
    "###..#######.",
    "...#.#....##.",
    "###.#..##.###",
];

fn assert_is_figure_i4(matrix: &BitMatrix) {
    assert_eq!(matrix.size(), 13);
    for (y, row) in FIGURE_I4.iter().enumerate() {
        assert_eq!(row.len(), 13);
        for (x, ch) in row.chars().enumerate() {
            assert_eq!(
                matrix.get(x as u32, y as u32),
                ch == '#',
                "module ({x}, {y}) differs from ISO Figure I.4"
            );
        }
    }
}

#[test]
fn iso_annex_i3_reproduces_figure_i4() {
    // Fully automatic: the encoder must pick numeric mode, version M2 (M1 does
    // not offer level L) and, by the 7.8.3.2 score, mask reference 01.
    let auto = encode_micro(b"01234567", options(MicroEcLevel::L, None, None)).unwrap();
    assert_is_figure_i4(&auto);

    // The same symbol with the version forced, and again with the mask forced to
    // the one I.3.5 selects.
    let forced_version = encode_micro(
        b"01234567",
        options(MicroEcLevel::L, Some(MicroVersion::M2), None),
    )
    .unwrap();
    assert_eq!(forced_version, auto);

    let forced_mask = encode_micro(
        b"01234567",
        options(MicroEcLevel::L, Some(MicroVersion::M2), Some(1)),
    )
    .unwrap();
    assert_eq!(forced_mask, auto);

    // I.3.5: the selected data masking pattern is referenced 01.
    // I.3.6: symbol number 1 (M2-L), mask 01.
    let (info, mask) = read_format(&auto);
    assert_eq!(mask, 1);
    assert_eq!(info.symbol_number, 1);
    assert_eq!(info.version, MicroVersion::M2);
    assert_eq!(info.ec, MicroEcLevel::L);

    // I.3.2 / I.3.3: the codewords the standard prints, recovered from the
    // finished symbol through the shared placement order.
    let (data, ec) = to_codewords(&extract_bits(&auto, MicroVersion::M2, mask), &info);
    assert_eq!(
        data,
        [0x40, 0x18, 0xAC, 0xC3, 0x00],
        "I.3.2 gives 01000000 00011000 10101100 11000011 00000000"
    );
    assert_eq!(
        ec,
        [0x86, 0x0D, 0x22, 0xAE, 0x30],
        "I.3.3 gives 10000110 00001101 00100010 10101110 00110000"
    );
}

// -- bit stream assembly, including the M1/M3 half codeword ------------------

#[test]
fn m1_bit_stream_and_half_codeword() {
    // ISO gives no worked M1 example, so the expectation is derived here from
    // Tables 2, 3 and 7 and clauses 7.4.10 / 7.4.11 for the payload "1234":
    //   mode indicator: M1 numeric is zero bits wide      -> (nothing)
    //   character count: 4 in 3 bits (Table 3)            -> 100
    //   "123" as a 3-digit group in 10 bits               -> 0001111011
    //   "4" as a 1-digit remainder in 4 bits              -> 0100
    //   terminator for M1 is 3 zero bits (Table 2)        -> 000
    //   total 20 bits, exactly the M1 data capacity (Table 7), so no padding
    //   bits and no pad codewords are needed.
    //   100 0001111011 0100 000 -> 10000011 11011010 0000
    // The final codeword is the 4-bit half codeword; it enters the
    // Reed-Solomon step zero-prefixed (value 0..=15, ISO Annex N.1 precedent)
    // and only those 4 bits are written into the symbol.
    let symbol = encode_micro(b"1234", options(MicroEcLevel::Detection, None, None)).unwrap();
    assert_eq!(symbol.size(), 11, "M1 is 11 modules across");

    let (info, mask) = read_format(&symbol);
    assert_eq!(info.version, MicroVersion::M1);
    assert_eq!(info.symbol_number, 0);
    assert!(info.has_half_final_data_codeword());

    let bits = extract_bits(&symbol, MicroVersion::M1, mask);
    assert_eq!(bits.len(), MicroVersion::M1.data_modules() as usize);
    assert_eq!(
        bits.len(),
        36,
        "20 data bits + 2 error correction codewords"
    );

    let (data, ec) = to_codewords(&bits, &info);
    assert_eq!(data, [0x83, 0xDA, 0x00]);
    assert_eq!(ec, rs_encode(&data, 2), "single block over 7.6");
}

#[test]
fn half_codeword_enters_reed_solomon_zero_prefixed() {
    // The convention for the 4-bit final data codeword of M1 and M3: it is a
    // value in 0..=15, zero-PREFIXED for the Reed-Solomon step, following the
    // ISO Annex N.1 precedent for the other 4-bit codeword the standard defines
    // ("The first 4-bit data codeword is prefixed with 0000 to make its length 8
    // bits for generating the error correction codewords"). Agreed workspace
    // wide with the qr-decode side; ISO prints no worked M1/M3 example, so this
    // test is the contract.
    //
    // "1234" cannot detect the difference — its half codeword is 0000, which is
    // the same byte either way. "12345" can:
    //   character count: 5 in 3 bits                      -> 101
    //   "123" as a 3-digit group in 10 bits               -> 0001111011
    //   "45" as a 2-digit remainder in 7 bits             -> 0101101
    //   20 bits exactly, so the terminator is omitted (7.4.10)
    //   101 0001111011 0101101 -> 10100011 11011010 1101
    // The half codeword is 1101, so it reaches Reed-Solomon as 0x0D, not 0xD0.
    let symbol = encode_micro(b"12345", options(MicroEcLevel::Detection, None, None)).unwrap();
    let (info, mask) = read_format(&symbol);
    let (data, ec) = to_codewords(&extract_bits(&symbol, MicroVersion::M1, mask), &info);

    assert_eq!(data, [0xA3, 0xDA, 0x0D]);
    assert_eq!(
        data[2] & 0xF0,
        0,
        "the half codeword is zero-prefixed, so its high nibble is clear"
    );
    // Pinned literally, not via rs_encode over the extracted data, so that the
    // error correction codewords themselves are the thing under test: the
    // zero-suffixed reading would give [0x6E, 0xC7] here.
    assert_eq!(ec, [0x14, 0x60]);
    assert_eq!(ec, rs_encode(&[0xA3, 0xDA, 0x0D], 2));
    assert_ne!(ec.as_slice(), rs_encode(&[0xA3, 0xDA, 0xD0], 2).as_slice());
}

#[test]
fn m3_half_codeword_is_a_pad_codeword() {
    // b"hi" in M3-L byte mode. Derived from Tables 2, 3 and 7 and 7.4.10/7.4.11:
    //   mode indicator: byte in M3 is 10 (2 bits)
    //   character count: 2 in 4 bits                      -> 0010
    //   'h' 0x68, 'i' 0x69                                -> 16 bits
    //   terminator for M3 is 7 zero bits
    //   = 29 bits; pad to the codeword boundary with 3 zero bits = 32 bits,
    //   then fill the remaining 8-bit data codewords with 11101100 / 00010001
    //   alternately up to 80 bits, then the final 4-bit data codeword is the
    //   0000 half pad codeword, reaching the 84 data bits of Table 7.
    let symbol = encode_micro(b"hi", options(MicroEcLevel::L, None, None)).unwrap();
    assert_eq!(symbol.size(), 15, "byte mode needs M3 at the earliest");

    let (info, mask) = read_format(&symbol);
    assert_eq!(info.version, MicroVersion::M3);
    assert_eq!(info.symbol_number, 3);

    let bits = extract_bits(&symbol, MicroVersion::M3, mask);
    assert_eq!(bits.len(), MicroVersion::M3.data_modules() as usize);

    let (data, ec) = to_codewords(&bits, &info);
    assert_eq!(
        data,
        [
            0x89, 0xA1, 0xA4, 0x00, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11, 0x00
        ]
    );
    assert_eq!(ec, rs_encode(&data, info.ec_codewords as usize));
}

#[test]
fn payload_running_into_the_half_codeword_is_bit_padded() {
    // 23 digits is the exact M3-L numeric capacity (Table 7): 2 + 5 + 77 = 84
    // bits, filling the symbol so that the terminator is omitted entirely
    // (7.4.10) and the last four bits are payload, not padding.
    let symbol = encode_micro(
        b"12345678901234567890123",
        options(MicroEcLevel::L, Some(MicroVersion::M3), None),
    )
    .unwrap();
    let (info, mask) = read_format(&symbol);
    let bits = extract_bits(&symbol, MicroVersion::M3, mask);
    let (data, ec) = to_codewords(&bits, &info);

    // Numeric mode indicator 00, count 23 = 10111, then "123" = 0001111011,
    // so the first codeword is 00 10111 0 = 0b0010_1110.
    assert_eq!(data[0], 0b0010_1110);
    assert_eq!(data.len(), 11);
    assert_eq!(ec, rs_encode(&data, info.ec_codewords as usize));
    assert_ne!(
        data[10], 0x00,
        "the half codeword carries payload here, not the 0000 pad"
    );
}

// -- capacities (ISO Table 7) ------------------------------------------------

#[test]
fn capacity_boundaries_match_table_7() {
    /// One Table 7 row: version, error correction, and the numeric,
    /// alphanumeric and byte data capacities. `None` where the table prints an
    /// em dash because the mode is unavailable in that version.
    type Row = (
        MicroVersion,
        MicroEcLevel,
        usize,
        Option<usize>,
        Option<usize>,
    );

    let table: [Row; 8] = [
        (MicroVersion::M1, MicroEcLevel::Detection, 5, None, None),
        (MicroVersion::M2, MicroEcLevel::L, 10, Some(6), None),
        (MicroVersion::M2, MicroEcLevel::M, 8, Some(5), None),
        (MicroVersion::M3, MicroEcLevel::L, 23, Some(14), Some(9)),
        (MicroVersion::M3, MicroEcLevel::M, 18, Some(11), Some(7)),
        (MicroVersion::M4, MicroEcLevel::L, 35, Some(21), Some(15)),
        (MicroVersion::M4, MicroEcLevel::M, 30, Some(18), Some(13)),
        (MicroVersion::M4, MicroEcLevel::Q, 21, Some(13), Some(9)),
    ];

    for (version, ec, numeric, alnum, byte) in table {
        // 'A' is alphanumeric but not numeric; 'a' is neither, so it forces byte
        // mode. Each payload therefore exercises exactly the intended mode.
        let cases = [(Some(numeric), b'7'), (alnum, b'A'), (byte, b'a')];
        for (capacity, filler) in cases {
            let Some(capacity) = capacity else {
                // The mode is unavailable: one character of it must be rejected
                // whatever the capacity would have been.
                let payload = vec![filler; 1];
                assert_eq!(
                    encode_micro(&payload, options(ec, Some(version), None)),
                    Err(MicroEncodeError::UnsupportedMode),
                    "{version:?} {ec:?} must reject {:?}",
                    filler as char
                );
                continue;
            };

            let exact = vec![filler; capacity];
            let symbol =
                encode_micro(&exact, options(ec, Some(version), None)).unwrap_or_else(|e| {
                    panic!(
                        "{version:?} {ec:?} must hold {capacity} x {:?}: {e}",
                        filler as char
                    )
                });
            assert_eq!(symbol.size(), version.size());
            let (info, _) = read_format(&symbol);
            assert_eq!((info.version, info.ec), (version, ec));

            let one_too_many = vec![filler; capacity + 1];
            assert_eq!(
                encode_micro(&one_too_many, options(ec, Some(version), None)),
                Err(MicroEncodeError::DataTooLarge),
                "{version:?} {ec:?} must reject {} x {:?}",
                capacity + 1,
                filler as char
            );
        }
    }
}

// -- mask handling -----------------------------------------------------------

#[test]
fn all_four_masks_can_be_forced() {
    for info in micro_symbols() {
        for mask in 0..4u8 {
            let symbol = encode_micro(b"1", options(info.ec, Some(info.version), Some(mask)))
                .unwrap_or_else(|e| panic!("{:?} {:?} mask {mask}: {e}", info.version, info.ec));
            let (read_info, read_mask) = read_format(&symbol);
            assert_eq!(read_mask, mask);
            assert_eq!(&read_info, info);

            // The mask really was applied: unmasking with it must reproduce a
            // codeword stream whose error correction codewords check out.
            let bits = extract_bits(&symbol, info.version, mask);
            let (data, ec) = to_codewords(&bits, info);
            assert_eq!(ec, rs_encode(&data, info.ec_codewords as usize));
        }
    }
}

#[test]
fn masks_produce_distinct_symbols() {
    let symbols: Vec<BitMatrix> = (0..4u8)
        .map(|mask| {
            encode_micro(
                b"01234567",
                options(MicroEcLevel::L, Some(MicroVersion::M2), Some(mask)),
            )
            .unwrap()
        })
        .collect();
    for i in 0..4 {
        for j in (i + 1)..4 {
            assert_ne!(symbols[i], symbols[j], "masks {i} and {j} coincide");
        }
    }
}

#[test]
fn out_of_range_mask_is_rejected() {
    for mask in [4u8, 7, 8, 255] {
        assert_eq!(
            encode_micro(b"1", options(MicroEcLevel::L, None, Some(mask))),
            Err(MicroEncodeError::InvalidMask)
        );
    }
}

#[test]
fn automatic_mask_maximises_the_iso_7_8_3_2_score() {
    // A spread of payloads across every symbol, so the argmax is exercised on
    // more than one winning mask.
    let payloads: [&[u8]; 5] = [b"1", b"01234567", b"999999", b"HELLO", b"micro qr"];
    for info in micro_symbols() {
        for payload in payloads {
            let Ok(symbol) = encode_micro(payload, options(info.ec, Some(info.version), None))
            else {
                continue; // payload does not fit or the mode is unavailable here
            };
            let (_, chosen) = read_format(&symbol);

            let scores: [u32; 4] = core::array::from_fn(|mask| {
                evaluation_score(&remask(&symbol, info.version, chosen, mask as u8))
            });
            let best = *scores.iter().max().unwrap();
            assert_eq!(
                scores[chosen as usize], best,
                "{:?} {:?} {payload:?}: chose mask {chosen} with {scores:?}",
                info.version, info.ec
            );
            // Ties go to the lowest mask id.
            let first_best = scores.iter().position(|&s| s == best).unwrap() as u8;
            assert_eq!(chosen, first_best);
        }
    }
}

// -- automatic version and mode selection ------------------------------------

#[test]
fn automatic_version_selection_takes_the_smallest_fit() {
    // Numeric at level L: M1 does not offer L, so selection starts at M2 and
    // steps up at each Table 7 numeric capacity (10 / 23 / 35).
    let expected = [
        (1usize, MicroVersion::M2),
        (10, MicroVersion::M2),
        (11, MicroVersion::M3),
        (23, MicroVersion::M3),
        (24, MicroVersion::M4),
        (35, MicroVersion::M4),
    ];
    for (len, version) in expected {
        let payload = vec![b'5'; len];
        let symbol = encode_micro(&payload, options(MicroEcLevel::L, None, None)).unwrap();
        assert_eq!(symbol.size(), version.size(), "{len} digits");
        // `MicroOptions::new` is the same fully-automatic configuration.
        assert_eq!(
            encode_micro(&payload, MicroOptions::new(MicroEcLevel::L)).unwrap(),
            symbol
        );
    }
    assert_eq!(
        encode_micro(&[b'5'; 36], options(MicroEcLevel::L, None, None)),
        Err(MicroEncodeError::DataTooLarge)
    );

    // Error detection only exists for M1, so it pins the version.
    let symbol = encode_micro(b"12345", options(MicroEcLevel::Detection, None, None)).unwrap();
    assert_eq!(symbol.size(), MicroVersion::M1.size());
    assert_eq!(
        encode_micro(b"123456", options(MicroEcLevel::Detection, None, None)),
        Err(MicroEncodeError::DataTooLarge)
    );

    // Level Q only exists for M4.
    let symbol = encode_micro(b"1", options(MicroEcLevel::Q, None, None)).unwrap();
    assert_eq!(symbol.size(), MicroVersion::M4.size());
}

#[test]
fn automatic_mode_selection_respects_availability() {
    // Alphanumeric needs M2, byte needs M3 (ISO Table 2), even though the
    // payloads are short enough for a smaller symbol by bit count.
    let alnum = encode_micro(b"AB", options(MicroEcLevel::L, None, None)).unwrap();
    assert_eq!(alnum.size(), MicroVersion::M2.size());

    let byte = encode_micro(b"ab", options(MicroEcLevel::L, None, None)).unwrap();
    assert_eq!(byte.size(), MicroVersion::M3.size());

    let binary = encode_micro(&[0x00, 0xFF, 0x80], options(MicroEcLevel::L, None, None)).unwrap();
    assert_eq!(binary.size(), MicroVersion::M3.size());

    // Numeric is preferred over alphanumeric for digits, which is why 10 digits
    // still fit M2-L.
    let numeric = encode_micro(b"0123456789", options(MicroEcLevel::L, None, None)).unwrap();
    assert_eq!(numeric.size(), MicroVersion::M2.size());

    // An empty payload is a zero-length numeric segment and fits M1.
    let empty = encode_micro(b"", options(MicroEcLevel::Detection, None, None)).unwrap();
    assert_eq!(empty.size(), MicroVersion::M1.size());
}

#[test]
fn unavailable_modes_and_levels_are_rejected() {
    // Modes the forced version does not offer.
    assert_eq!(
        encode_micro(
            b"A",
            options(MicroEcLevel::Detection, Some(MicroVersion::M1), None)
        ),
        Err(MicroEncodeError::UnsupportedMode)
    );
    assert_eq!(
        encode_micro(b"a", options(MicroEcLevel::L, Some(MicroVersion::M2), None)),
        Err(MicroEncodeError::UnsupportedMode)
    );
    // ... and with the version left to the encoder, when no version can help.
    assert_eq!(
        encode_micro(b"a", options(MicroEcLevel::Detection, None, None)),
        Err(MicroEncodeError::UnsupportedMode)
    );

    // Error-correction selections the forced version does not offer (Table 13).
    for (version, ec) in [
        (MicroVersion::M1, MicroEcLevel::L),
        (MicroVersion::M1, MicroEcLevel::M),
        (MicroVersion::M1, MicroEcLevel::Q),
        (MicroVersion::M2, MicroEcLevel::Detection),
        (MicroVersion::M2, MicroEcLevel::Q),
        (MicroVersion::M3, MicroEcLevel::Detection),
        (MicroVersion::M3, MicroEcLevel::Q),
        (MicroVersion::M4, MicroEcLevel::Detection),
    ] {
        assert_eq!(
            encode_micro(b"1", options(ec, Some(version), None)),
            Err(MicroEncodeError::UnsupportedEcLevel),
            "{version:?} must not offer {ec:?}"
        );
    }
}

// -- symbol structure --------------------------------------------------------

#[test]
fn function_patterns_are_correct_for_every_version() {
    for info in micro_symbols() {
        let version = info.version;
        let size = version.size();
        let symbol = encode_micro(b"1", options(info.ec, Some(version), None)).unwrap();
        assert_eq!(symbol.size(), size);

        // Single 7 x 7 finder at the top-left, 1:1:3:1:1 (ISO 5.3.3.2).
        for y in 0..7u32 {
            for x in 0..7u32 {
                let expected = x == 0
                    || x == 6
                    || y == 0
                    || y == 6
                    || ((2..=4).contains(&x) && (2..=4).contains(&y));
                assert_eq!(
                    symbol.get(x, y),
                    expected,
                    "finder ({x}, {y}) in {version:?}"
                );
            }
        }

        // One-module separator along row 7 and column 7 (ISO 5.3.4).
        for i in 0..8u32 {
            assert!(!symbol.get(i, 7), "separator ({i}, 7) in {version:?}");
            assert!(!symbol.get(7, i), "separator (7, {i}) in {version:?}");
        }

        // Timing patterns run edge to edge along row 0 and column 0 (ISO 5.3.5).
        for i in 8..size {
            assert_eq!(
                symbol.get(i, 0),
                i % 2 == 0,
                "timing ({i}, 0) in {version:?}"
            );
            assert_eq!(
                symbol.get(0, i),
                i % 2 == 0,
                "timing (0, {i}) in {version:?}"
            );
        }

        // No alignment pattern anywhere: every module outside the function map
        // is a data module and the placement order visits each exactly once.
        let placed: Vec<(u32, u32)> = micro_data_modules(version).collect();
        assert_eq!(placed.len(), version.data_modules() as usize);
        let mut seen = vec![false; (size * size) as usize];
        for (x, y) in placed {
            assert!(!micro_is_function_module(version, x, y));
            let idx = (y * size + x) as usize;
            assert!(!seen[idx], "module ({x}, {y}) placed twice");
            seen[idx] = true;
        }
    }
}

#[test]
fn every_symbol_number_round_trips_through_the_format_information() {
    for info in micro_symbols() {
        let symbol = encode_micro(b"1", options(info.ec, Some(info.version), None)).unwrap();
        let (read_info, _) = read_format(&symbol);
        assert_eq!(&read_info, info, "Table 13 symbol {}", info.symbol_number);
    }
}
