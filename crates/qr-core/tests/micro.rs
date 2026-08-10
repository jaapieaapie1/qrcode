//! Micro QR primitives, checked against ISO/IEC 18004:2024.
//!
//! Every constant asserted here was read out of the standard's tables and
//! figures rather than from memory; the clause or table number is cited on each
//! test. The strongest check is [`iso_annex_i3_reproduces_figure_i4`], which
//! rebuilds the worked M2-L example module for module.

#![cfg(feature = "micro")]

use qr_core::micro::{
    MICRO_FORMAT_INFO_POSITIONS, MICRO_FORMAT_MASK, MICRO_MASK_TO_QR, MicroEcLevel, MicroMode,
    MicroVersion, micro_data_modules, micro_format_info_decode, micro_format_info_encode,
    micro_format_info_read, micro_format_info_write, micro_is_function_module, micro_mask_bit,
    micro_symbol_info, micro_symbol_info_from_number, micro_symbols,
};
use qr_core::{BitMatrix, EcLevel, mask_bit, reed_solomon::rs_encode};

/// ISO 5.3.2.2 and Table 1 column A: M1..M4 are 11, 13, 15 and 17 modules square.
#[test]
fn version_sizes_match_iso_5_3_2_2() {
    assert_eq!(MicroVersion::M1.size(), 11);
    assert_eq!(MicroVersion::M2.size(), 13);
    assert_eq!(MicroVersion::M3.size(), 15);
    assert_eq!(MicroVersion::M4.size(), 17);

    for v in MicroVersion::ALL {
        assert_eq!(v.size(), 9 + 2 * v.number() as u32);
        assert_eq!(MicroVersion::from_size(v.size()), Some(v));
        assert_eq!(MicroVersion::from_number(v.number()), Some(v));
    }
    // Full-QR sizes and nonsense must not resolve to a micro version.
    for bad in [0, 1, 9, 10, 12, 14, 16, 18, 21, 25, 177] {
        assert_eq!(MicroVersion::from_size(bad), None, "size {bad}");
    }
    assert_eq!(MicroVersion::from_number(0), None);
    assert_eq!(MicroVersion::from_number(5), None);
}

/// ISO Table 1: columns B (function pattern modules), C (format information
/// modules), D (data modules) and E (data capacity in codewords).
///
/// The table is reproduced here and cross-checked against the function map, so
/// the map and the capacity numbers cannot drift apart.
#[test]
fn table_1_module_accounting() {
    // (version, function pattern modules B, format modules C, data modules D,
    //  codeword capacity E)
    let table = [
        (MicroVersion::M1, 70, 15, 36, 5),
        (MicroVersion::M2, 74, 15, 80, 10),
        (MicroVersion::M3, 78, 15, 132, 17),
        (MicroVersion::M4, 82, 15, 192, 24),
    ];

    for (v, function_modules, format_modules, data_modules, codewords) in table {
        let size = v.size();

        // Count what the function map actually reserves, splitting out the
        // format information so both of ISO Table 1's columns B and C are
        // verified independently.
        let mut reserved = 0u32;
        for y in 0..size {
            for x in 0..size {
                if micro_is_function_module(v, x, y) {
                    reserved += 1;
                }
            }
        }
        let format_count = MICRO_FORMAT_INFO_POSITIONS.len() as u32;
        assert_eq!(format_count, format_modules, "{v:?} format modules (col C)");
        assert_eq!(
            reserved - format_count,
            function_modules,
            "{v:?} function pattern modules (col B)"
        );

        // D = A^2 - B - C.
        assert_eq!(size * size - reserved, data_modules, "{v:?} data modules");
        assert_eq!(v.data_modules(), data_modules, "{v:?} data_modules()");
        assert_eq!(v.total_codewords(), codewords, "{v:?} total codewords");
    }
}

/// ISO Table 13: the eight `(symbol number, version, error correction level)`
/// triples, and the fact that no other combination exists.
#[test]
fn table_13_symbol_numbers() {
    let table = [
        (0u8, MicroVersion::M1, MicroEcLevel::Detection),
        (1, MicroVersion::M2, MicroEcLevel::L),
        (2, MicroVersion::M2, MicroEcLevel::M),
        (3, MicroVersion::M3, MicroEcLevel::L),
        (4, MicroVersion::M3, MicroEcLevel::M),
        (5, MicroVersion::M4, MicroEcLevel::L),
        (6, MicroVersion::M4, MicroEcLevel::M),
        (7, MicroVersion::M4, MicroEcLevel::Q),
    ];

    for (number, version, ec) in table {
        let info = micro_symbol_info(version, ec).expect("valid combination");
        assert_eq!(info.symbol_number, number, "{version:?}-{ec:?}");
        let by_number = micro_symbol_info_from_number(number).expect("valid symbol number");
        assert_eq!(by_number, info);
    }
    assert_eq!(micro_symbol_info_from_number(8), None);

    // Combinations absent from Table 13 must be rejected, not guessed at.
    for version in MicroVersion::ALL {
        for ec in MicroEcLevel::ALL {
            let listed = table.iter().any(|&(_, v, e)| v == version && e == ec);
            assert_eq!(
                micro_symbol_info(version, ec).is_some(),
                listed,
                "{version:?}-{ec:?} availability"
            );
            assert_eq!(
                version.ec_levels().contains(&ec),
                listed,
                "{version:?}.ec_levels() should agree with Table 13"
            );
        }
    }

    // ISO 7.5.1: level H is never available in a micro symbol.
    assert_eq!(MicroEcLevel::from_ec_level(EcLevel::H), None);
    for ec in [EcLevel::L, EcLevel::M, EcLevel::Q] {
        let micro = MicroEcLevel::from_ec_level(ec).expect("L/M/Q map across");
        assert_eq!(micro.ec_level(), Some(ec));
    }
    // "Error detection only" has no full-QR counterpart.
    assert_eq!(MicroEcLevel::Detection.ec_level(), None);
}

/// ISO Table 7 (data codewords and data bits) and ISO Table 9 (total codewords,
/// error correction codewords, block structure `(c, k, r)` and misdecode
/// protection `p` from 7.5.1), plus the 4-bit final codeword rule of ISO 7.4.11.
#[test]
fn table_7_and_table_9_capacities() {
    // (symbol number, total c, data k, ec, data bits, capacity r, protection p)
    let table = [
        (0u8, 5u8, 3u8, 2u8, 20u16, 0u8, 2u8),
        (1, 10, 5, 5, 40, 1, 3),
        (2, 10, 4, 6, 32, 2, 2),
        (3, 17, 11, 6, 84, 2, 2),
        (4, 17, 9, 8, 68, 4, 0),
        (5, 24, 16, 8, 128, 3, 2),
        (6, 24, 14, 10, 112, 5, 0),
        (7, 24, 10, 14, 80, 7, 0),
    ];

    for (number, total, data, ec, data_bits, capacity, protection) in table {
        let info = micro_symbol_info_from_number(number).expect("valid symbol number");
        assert_eq!(info.total_codewords, total, "symbol {number} c");
        assert_eq!(info.data_codewords, data, "symbol {number} k");
        assert_eq!(info.ec_codewords, ec, "symbol {number} ec");
        assert_eq!(info.data_bits, data_bits, "symbol {number} data bits");
        assert_eq!(
            info.error_correction_capacity, capacity,
            "symbol {number} r"
        );
        assert_eq!(info.misdecode_protection, protection, "symbol {number} p");

        // c = k + ec.
        assert_eq!(
            info.data_codewords + info.ec_codewords,
            info.total_codewords
        );
        assert_eq!(
            info.total_codewords,
            info.version.total_codewords(),
            "symbol {number} agrees with Table 1 column E"
        );

        // ISO 7.4.11: every codeword is 8 bits except the final data codeword of
        // M1 and M3, which is 4.
        let expected_final = match info.version {
            MicroVersion::M1 | MicroVersion::M3 => 4,
            MicroVersion::M2 | MicroVersion::M4 => 8,
        };
        assert_eq!(info.final_data_codeword_bits(), expected_final);
        assert_eq!(
            info.has_half_final_data_codeword(),
            expected_final == 4,
            "symbol {number} half codeword"
        );

        // Data bits = full data codewords plus the (possibly half) final one.
        assert_eq!(
            (info.data_codewords as u16 - 1) * 8 + info.final_data_codeword_bits() as u16,
            info.data_bits,
            "symbol {number} data bits vs codeword widths"
        );

        // The whole codeword stream fills the symbol exactly: micro symbols
        // have no remainder bits (ISO Table 1).
        assert_eq!(
            info.data_bits as u32 + info.ec_codewords as u32 * 8,
            info.version.data_modules(),
            "symbol {number} fills the data modules exactly"
        );

        // ISO 7.5.1 Formula (3) with e = 0: 2t <= d - p.
        assert!(
            2 * info.error_correction_capacity + info.misdecode_protection <= info.ec_codewords,
            "symbol {number} violates 2t + p <= d"
        );
    }

    assert_eq!(micro_symbols().len(), table.len());
}

/// ISO Table 2: mode indicator widths (0, 1, 2, 3 bits), the indicator values
/// per version, mode availability, and terminator lengths (3, 5, 7, 9 bits).
#[test]
fn table_2_mode_indicators_and_terminators() {
    assert_eq!(MicroVersion::M1.mode_indicator_bits(), 0);
    assert_eq!(MicroVersion::M2.mode_indicator_bits(), 1);
    assert_eq!(MicroVersion::M3.mode_indicator_bits(), 2);
    assert_eq!(MicroVersion::M4.mode_indicator_bits(), 3);

    assert_eq!(MicroVersion::M1.terminator_bits(), 3);
    assert_eq!(MicroVersion::M2.terminator_bits(), 5);
    assert_eq!(MicroVersion::M3.terminator_bits(), 7);
    assert_eq!(MicroVersion::M4.terminator_bits(), 9);

    // Table 2, "NA" cells included: M1 is numeric only (7.4.5 — alphanumeric is
    // not available in M1); byte and Kanji are unavailable in M1 and M2 (7.4.6,
    // 7.4.7).
    let expected: [[Option<u8>; 4]; 4] = [
        [Some(0), None, None, None],
        [Some(0), Some(1), None, None],
        [Some(0), Some(1), Some(2), Some(3)],
        [Some(0), Some(1), Some(2), Some(3)],
    ];
    for (vi, version) in MicroVersion::ALL.into_iter().enumerate() {
        for (mi, mode) in MicroMode::ALL.into_iter().enumerate() {
            match expected[vi][mi] {
                Some(value) => {
                    assert!(mode.is_available(version), "{version:?} {mode:?}");
                    assert_eq!(
                        mode.indicator(version),
                        Some((value, version.mode_indicator_bits())),
                        "{version:?} {mode:?} indicator"
                    );
                }
                None => {
                    assert!(!mode.is_available(version), "{version:?} {mode:?}");
                    assert_eq!(mode.indicator(version), None);
                    assert_eq!(mode.char_count_bits(version), None);
                }
            }
        }
    }

    // The indicator must fit in its declared width.
    for version in MicroVersion::ALL {
        for mode in MicroMode::ALL {
            if let Some((value, bits)) = mode.indicator(version) {
                assert!(
                    bits == 0 && value == 0 || (value as u32) < (1u32 << bits),
                    "{version:?} {mode:?} indicator {value} does not fit in {bits} bits"
                );
            }
        }
    }
}

/// ISO Table 3: character count indicator widths for the micro versions.
#[test]
fn table_3_character_count_indicator_widths() {
    // Rows M1..M4, columns numeric / alphanumeric / byte / Kanji; `None` = "NA".
    let table: [[Option<u8>; 4]; 4] = [
        [Some(3), None, None, None],
        [Some(4), Some(3), None, None],
        [Some(5), Some(4), Some(4), Some(3)],
        [Some(6), Some(5), Some(5), Some(4)],
    ];
    for (vi, version) in MicroVersion::ALL.into_iter().enumerate() {
        for (mi, mode) in MicroMode::ALL.into_iter().enumerate() {
            assert_eq!(
                mode.char_count_bits(version),
                table[vi][mi],
                "{version:?} {mode:?} char count width"
            );
        }
    }
}

/// ISO Table 10: the four micro data mask references and their conditions,
/// evaluated directly from the standard's formulae rather than via the QR
/// mapping the implementation uses.
// The mask conditions below are transcribed verbatim from ISO Table 10, in the
// standard's own `mod` notation, so that the test reads as a direct quotation of
// the specification rather than a paraphrase of `mask.rs`. Rewriting them as
// `is_multiple_of` would make the two identical and lose that independence.
#[allow(clippy::manual_is_multiple_of)]
#[test]
fn table_10_micro_data_masks() {
    // Table 10 pairs micro reference 00/01/10/11 with QR reference
    // 001/100/110/111 respectively.
    assert_eq!(MICRO_MASK_TO_QR, [1, 4, 6, 7]);

    for y in 0..40u32 {
        for x in 0..40u32 {
            let i = y as u64; // row
            let j = x as u64; // column
            let expected = [
                i % 2 == 0,
                ((i / 2) + (j / 3)) % 2 == 0,
                ((i * j) % 2 + (i * j) % 3) % 2 == 0,
                ((i + j) % 2 + (i * j) % 3) % 2 == 0,
            ];
            for (id, want) in expected.into_iter().enumerate() {
                assert_eq!(
                    micro_mask_bit(id as u8, x, y),
                    want,
                    "micro mask {id} at ({x}, {y})"
                );
                // And it must agree with the shared QR predicate it maps onto.
                assert_eq!(
                    micro_mask_bit(id as u8, x, y),
                    mask_bit(MICRO_MASK_TO_QR[id], x, y)
                );
            }
        }
    }

    // Out-of-range mask ids are inert rather than panicking.
    for bad in [4u8, 7, 255] {
        assert!(!micro_mask_bit(bad, 3, 5));
    }
}

/// ISO Table C.1, "Sequence after masking (micro QR code symbols)": all 32
/// valid 15-bit format information values, transcribed from the standard.
const TABLE_C1_MICRO: [u16; 32] = [
    0x4445, 0x4172, 0x4E2B, 0x4B1C, 0x55AE, 0x5099, 0x5FC0, 0x5AF7, 0x6793, 0x62A4, 0x6DFD, 0x68CA,
    0x7678, 0x734F, 0x7C16, 0x7921, 0x06DE, 0x03E9, 0x0CB0, 0x0987, 0x1735, 0x1202, 0x1D5B, 0x186C,
    0x2508, 0x203F, 0x2F66, 0x2A51, 0x34E3, 0x31D4, 0x3E8D, 0x3BBA,
];

/// ISO 7.9.2 and Annex C: the 5 data bits are the 3-bit symbol number followed
/// by the 2-bit micro mask reference, BCH(15,5) encoded and XORed with
/// `100010001000101`. Checked against every row of Table C.1.
#[test]
fn annex_c1_micro_format_information_table() {
    assert_eq!(MICRO_FORMAT_MASK, 0x4445);

    for info in micro_symbols() {
        for mask in 0..4u8 {
            let index = ((info.symbol_number as usize) << 2) | mask as usize;
            let encoded =
                micro_format_info_encode(info.version, info.ec, mask).expect("valid symbol");
            assert_eq!(
                encoded, TABLE_C1_MICRO[index],
                "symbol {} mask {mask} (Table C.1 row {index})",
                info.symbol_number
            );
            assert_eq!(encoded & 0x7FFF, encoded, "format info must be 15 bits");
        }
    }

    // All 32 values are distinct, and the mask guarantees none is all-zero
    // (the stated purpose of the XOR in 7.9.2).
    for (a, &va) in TABLE_C1_MICRO.iter().enumerate() {
        assert_ne!(va, 0);
        for &vb in TABLE_C1_MICRO.iter().skip(a + 1) {
            assert_ne!(va, vb);
        }
    }

    // Masks outside 0..=3 are rejected rather than silently truncated.
    assert_eq!(
        micro_format_info_encode(MicroVersion::M4, MicroEcLevel::Q, 4),
        None
    );
    // So are combinations Table 13 does not list.
    assert_eq!(
        micro_format_info_encode(MicroVersion::M1, MicroEcLevel::L, 0),
        None
    );
}

/// ISO Annex C.3: the BCH(15,5) code has minimum distance 7, so up to 3 bit
/// errors are correctable. Every valid word must round-trip, and every 1-, 2-
/// and 3-bit corruption of every valid word must be recovered exactly.
#[test]
fn annex_c3_format_information_error_correction() {
    for info in micro_symbols() {
        for mask in 0..4u8 {
            let clean = micro_format_info_encode(info.version, info.ec, mask).unwrap();

            let (got, got_mask) = micro_format_info_decode(clean).expect("clean word decodes");
            assert_eq!(got, *info);
            assert_eq!(got_mask, mask);

            // One bit.
            for a in 0..15 {
                let (d, m) = micro_format_info_decode(clean ^ (1 << a))
                    .unwrap_or_else(|| panic!("1-bit error at {a} should be corrected"));
                assert_eq!((d, m), (*info, mask), "1-bit error at {a}");
            }
            // Two bits.
            for a in 0..15 {
                for b in (a + 1)..15 {
                    let corrupt = clean ^ (1 << a) ^ (1 << b);
                    let (d, m) = micro_format_info_decode(corrupt)
                        .unwrap_or_else(|| panic!("2-bit error at {a},{b}"));
                    assert_eq!((d, m), (*info, mask), "2-bit error at {a},{b}");
                }
            }
            // Three bits.
            for a in 0..15 {
                for b in (a + 1)..15 {
                    for c in (b + 1)..15 {
                        let corrupt = clean ^ (1 << a) ^ (1 << b) ^ (1 << c);
                        let (d, m) = micro_format_info_decode(corrupt)
                            .unwrap_or_else(|| panic!("3-bit error at {a},{b},{c}"));
                        assert_eq!((d, m), (*info, mask), "3-bit error at {a},{b},{c}");
                    }
                }
            }
        }
    }
}

/// ISO 7.9.2 Figure 26: bit 0 sits at (8, 1) and the strip runs down column 8 to
/// (8, 8), then leftwards along row 8 to (1, 8) holding bit 14.
#[test]
fn figure_26_format_information_positions() {
    let mut expected = [(0u32, 0u32); 15];
    for (bit, slot) in expected.iter_mut().enumerate().take(8) {
        *slot = (8, bit as u32 + 1);
    }
    for (n, slot) in expected.iter_mut().skip(8).enumerate() {
        *slot = (7 - n as u32, 8);
    }
    assert_eq!(MICRO_FORMAT_INFO_POSITIONS, expected);

    // Every format module must also be reported as reserved by the function map,
    // for all four versions.
    for version in MicroVersion::ALL {
        for &(x, y) in &MICRO_FORMAT_INFO_POSITIONS {
            assert!(
                micro_is_function_module(version, x, y),
                "{version:?} ({x}, {y}) should be reserved"
            );
        }
    }

    // Read-back must invert write, bit for bit.
    for &value in &TABLE_C1_MICRO {
        let mut matrix = BitMatrix::new(MicroVersion::M2.size());
        micro_format_info_write(&mut matrix, value);
        assert_eq!(micro_format_info_read(&matrix), value);
    }
}

/// ISO 5.3.3.2, 5.3.4, 5.3.5 and 7.9.2: the reserved regions of a micro symbol
/// are the finder plus separator, the full timing row and column, and the format
/// information strip. Nothing else.
#[test]
fn micro_function_map_regions() {
    for version in MicroVersion::ALL {
        let size = version.size();
        for y in 0..size {
            for x in 0..size {
                let finder_and_separator = x < 8 && y < 8;
                let timing = x == 0 || y == 0;
                let format = (x == 8 && (1..=8).contains(&y)) || (y == 8 && (1..=8).contains(&x));
                assert_eq!(
                    micro_is_function_module(version, x, y),
                    finder_and_separator || timing || format,
                    "{version:?} ({x}, {y})"
                );
            }
        }
        // Out of range is `false`, never a panic.
        assert!(!micro_is_function_module(version, size, 0));
        assert!(!micro_is_function_module(version, 0, size));
        assert!(!micro_is_function_module(version, u32::MAX, u32::MAX));
    }

    // A micro symbol has no alignment pattern and no fixed dark module: the
    // centre of an M4 symbol, where a QR alignment pattern would sit, is data.
    assert!(!micro_is_function_module(MicroVersion::M4, 12, 12));
}

/// ISO 7.7.3: the placement order must visit every data module exactly once and
/// no function module at all, and must produce exactly as many positions as
/// ISO Table 1 column D lists.
#[test]
fn placement_iterator_covers_every_data_module_once() {
    for version in MicroVersion::ALL {
        let size = version.size();
        let positions: Vec<(u32, u32)> = micro_data_modules(version).collect();

        assert_eq!(
            positions.len() as u32,
            version.data_modules(),
            "{version:?} position count"
        );

        let mut seen = vec![false; (size * size) as usize];
        for &(x, y) in &positions {
            assert!(x < size && y < size, "{version:?} ({x}, {y}) out of bounds");
            assert!(
                !micro_is_function_module(version, x, y),
                "{version:?} ({x}, {y}) is a function module"
            );
            let idx = (y * size + x) as usize;
            assert!(!seen[idx], "{version:?} ({x}, {y}) visited twice");
            seen[idx] = true;
        }
        // Conversely, every non-function module must have been visited.
        for y in 0..size {
            for x in 0..size {
                if !micro_is_function_module(version, x, y) {
                    assert!(
                        seen[(y * size + x) as usize],
                        "{version:?} ({x}, {y}) never visited"
                    );
                }
            }
        }

        // Placement starts at the bottom-right corner and works upwards.
        assert_eq!(positions[0], (size - 1, size - 1), "{version:?} start");
    }
}

/// ISO 7.7.3: "the sole exception is that D3 in a version M1 symbol, D11 in a
/// version M3-L symbol and D9 in a version M3-M symbol is a 2 x 2 square
/// 4-module block." Those are exactly the 4-bit final data codewords.
#[test]
fn half_codeword_forms_a_two_by_two_block() {
    // (version, ec, 1-based index of the half codeword within the data stream)
    let cases = [
        (MicroVersion::M1, MicroEcLevel::Detection, 3usize),
        (MicroVersion::M3, MicroEcLevel::L, 11),
        (MicroVersion::M3, MicroEcLevel::M, 9),
    ];

    for (version, ec, dn) in cases {
        let info = micro_symbol_info(version, ec).unwrap();
        assert!(info.has_half_final_data_codeword());
        assert_eq!(
            info.data_codewords as usize, dn,
            "{version:?}-{ec:?} half codeword should be D{dn}"
        );

        let positions: Vec<(u32, u32)> = micro_data_modules(version).collect();
        // The half codeword occupies the four bits after (dn - 1) full codewords.
        let start = (dn - 1) * 8;
        let block = &positions[start..start + 4];

        let xs: Vec<u32> = block.iter().map(|&(x, _)| x).collect();
        let ys: Vec<u32> = block.iter().map(|&(_, y)| y).collect();
        let (minx, maxx) = (*xs.iter().min().unwrap(), *xs.iter().max().unwrap());
        let (miny, maxy) = (*ys.iter().min().unwrap(), *ys.iter().max().unwrap());
        assert_eq!(maxx - minx, 1, "{version:?}-{ec:?} block width");
        assert_eq!(maxy - miny, 1, "{version:?}-{ec:?} block height");

        // All four corners of that 2 x 2 square, no repeats.
        let mut corners: Vec<(u32, u32)> = block.to_vec();
        corners.sort_unstable();
        assert_eq!(
            corners,
            vec![(minx, miny), (minx, maxy), (maxx, miny), (maxx, maxy)],
            "{version:?}-{ec:?} is not a 2 x 2 block"
        );
    }

    // M2 and M4 have no half codeword.
    for (version, ec) in [
        (MicroVersion::M2, MicroEcLevel::L),
        (MicroVersion::M4, MicroEcLevel::Q),
    ] {
        assert!(
            !micro_symbol_info(version, ec)
                .unwrap()
                .has_half_final_data_codeword()
        );
    }
}

/// ISO Annex I.3, the worked example: the string "01234567" encoded into an
/// M2-L symbol. Rebuilds the symbol from core primitives and compares it module
/// for module against Figure I.4, the printed final symbol.
///
/// The reference grid below was read off Figure I.4 (`#` = dark). It exercises,
/// in one shot: Reed-Solomon over the micro data codewords, the placement
/// iterator, micro data mask 01, the format information value and the format
/// information bit positions.
#[test]
fn iso_annex_i3_reproduces_figure_i4() {
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

    let version = MicroVersion::M2;
    let ec = MicroEcLevel::L;
    let info = micro_symbol_info(version, ec).unwrap();
    let size = version.size();

    // I.3.2 Step 1: the data bit stream, divided into five 8-bit codewords.
    //   0 1000 0000001100 0101011001 1000011 00000  + 3 padding bits
    //   01000000 00011000 10101100 11000011 00000000
    let data: [u8; 5] = [0x40, 0x18, 0xAC, 0xC3, 0x00];
    assert_eq!(data.len(), info.data_codewords as usize);

    // I.3.3 Step 2: five error correction codewords, generated by the shared
    // Reed-Solomon implementation — micro symbols are a single block (7.6).
    let ec_codewords = rs_encode(&data, info.ec_codewords as usize);
    assert_eq!(
        ec_codewords.as_slice(),
        &[0x86, 0x0D, 0x22, 0xAE, 0x30],
        "Annex I.3.3 error correction codewords"
    );

    let mut stream = Vec::new();
    stream.extend_from_slice(&data);
    stream.extend_from_slice(&ec_codewords);
    assert_eq!(stream.len(), info.total_codewords as usize);

    // I.3.4 Step 3: place the codeword stream, most significant bit first.
    let mut matrix = BitMatrix::new(size);
    draw_micro_function_patterns(version, &mut matrix);

    let bits = stream.len() * 8;
    assert_eq!(bits as u32, version.data_modules());

    // I.3.5 Step 4: the selected data mask is micro reference 01.
    let mask = 1u8;
    for (n, (x, y)) in micro_data_modules(version).enumerate() {
        let bit = (stream[n / 8] >> (7 - (n % 8))) & 1 == 1;
        matrix.set(x, y, bit ^ micro_mask_bit(mask, x, y));
    }

    // I.3.6 Step 5: format information for symbol number 1, mask 01.
    let format = micro_format_info_encode(version, ec, mask).unwrap();
    assert_eq!(format, 0x5099, "Annex I.3.6 gives 101000010011001");
    micro_format_info_write(&mut matrix, format);

    // I.3.7 Step 6: compare the finished symbol with Figure I.4.
    for (y, row) in FIGURE_I4.iter().enumerate() {
        assert_eq!(row.len(), size as usize);
        for (x, ch) in row.chars().enumerate() {
            assert_eq!(
                matrix.get(x as u32, y as u32),
                ch == '#',
                "module ({x}, {y}) differs from Figure I.4"
            );
        }
    }

    // And the symbol reads back: format information recovers the version, the
    // error correction level and the mask.
    let (read_info, read_mask) =
        micro_format_info_decode(micro_format_info_read(&matrix)).expect("format decodes");
    assert_eq!(read_info, info);
    assert_eq!(read_mask, mask);

    // Unmasking through the same placement iterator returns the codewords,
    // which is the property the decoder in step 5c will rely on.
    let mut recovered = vec![0u8; stream.len()];
    for (n, (x, y)) in micro_data_modules(version).enumerate() {
        if matrix.get(x, y) ^ micro_mask_bit(mask, x, y) {
            recovered[n / 8] |= 1 << (7 - (n % 8));
        }
    }
    assert_eq!(recovered, stream);
}

/// Draws the finder pattern, its separator and both timing patterns for a micro
/// symbol (ISO 5.3.3.1, 5.3.4, 5.3.5). Test-local: `qr-core` deliberately
/// exposes the function *map*, not a renderer.
fn draw_micro_function_patterns(version: MicroVersion, matrix: &mut BitMatrix) {
    let size = version.size();

    // 7 x 7 finder: dark outer ring, light 5 x 5, dark 3 x 3 core.
    for y in 0..7u32 {
        for x in 0..7u32 {
            let outer = x == 0 || x == 6 || y == 0 || y == 6;
            let core = (2..=4).contains(&x) && (2..=4).contains(&y);
            matrix.set(x, y, outer || core);
        }
    }
    // The separator (row 7 and column 7) is already light.

    // Timing patterns: row 0 and column 0 from module 8 to the symbol edge,
    // starting and ending dark.
    for i in 8..size {
        matrix.set(i, 0, i % 2 == 0);
        matrix.set(0, i, i % 2 == 0);
    }
}

/// Pins the documented hazard on `micro_format_info_read`/`decode`: garbage is
/// NOT reliably rejected. The all-light read (`0`) decodes successfully, so
/// decoders must gate symbol validity on RS syndromes, not on format decode.
#[test]
fn format_decode_accepts_garbage_by_design() {
    assert!(
        micro_format_info_decode(0).is_some(),
        "all-light format read is within BCH correction distance of a valid \
         word; if this ever starts failing, the doc warning on \
         micro_format_info_read is stale"
    );
}
