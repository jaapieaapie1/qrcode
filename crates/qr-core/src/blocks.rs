//! Error-correction block structure per (version, EC level) — ISO Table 9.

use crate::{EcLevel, Version};

/// Describes how a symbol's codewords are split into Reed-Solomon blocks.
///
/// A symbol may use up to two groups of blocks; every block in a group has the
/// same number of data codewords, and every block in the symbol has the same
/// number of EC codewords.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EcBlocks {
    /// EC codewords per block.
    pub ec_per_block: u16,
    /// Number of blocks in group 1.
    pub group1_blocks: u16,
    /// Data codewords in each group-1 block.
    pub group1_data: u16,
    /// Number of blocks in group 2.
    pub group2_blocks: u16,
    /// Data codewords in each group-2 block.
    pub group2_data: u16,
}

impl EcBlocks {
    /// Total number of Reed-Solomon blocks.
    #[inline]
    pub fn total_blocks(self) -> u16 {
        self.group1_blocks + self.group2_blocks
    }

    /// Total number of data codewords across all blocks.
    #[inline]
    pub fn total_data_codewords(self) -> u16 {
        self.group1_blocks * self.group1_data + self.group2_blocks * self.group2_data
    }

    /// Total number of error-correction codewords across all blocks.
    #[inline]
    pub fn total_ec_codewords(self) -> u16 {
        self.total_blocks() * self.ec_per_block
    }

    /// Total number of codewords (data + EC). Must equal
    /// [`Version::total_codewords`] for a valid symbol.
    #[inline]
    pub fn total_codewords(self) -> u16 {
        self.total_data_codewords() + self.total_ec_codewords()
    }
}

/// Returns the EC block structure for a symbol.
pub fn ec_blocks(version: Version, ec: EcLevel) -> EcBlocks {
    let (e, g1b, g1d, g2b, g2d) = TABLE[version.number() as usize - 1][ec.index()];
    EcBlocks {
        ec_per_block: e as u16,
        group1_blocks: g1b as u16,
        group1_data: g1d as u16,
        group2_blocks: g2b as u16,
        group2_data: g2d as u16,
    }
}

/// One ISO Table 9 row:
/// `(ec_per_block, group1_blocks, group1_data, group2_blocks, group2_data)`.
type BlockSpec = (u8, u8, u8, u8, u8);

/// Raw ISO Table 9, indexed by `[version - 1][ec_level_index]` where the level
/// order is `L, M, Q, H`.
#[rustfmt::skip]
const TABLE: [[BlockSpec; 4]; 40] = [
    // v1
    [(7, 1, 19, 0, 0), (10, 1, 16, 0, 0), (13, 1, 13, 0, 0), (17, 1, 9, 0, 0)],
    // v2
    [(10, 1, 34, 0, 0), (16, 1, 28, 0, 0), (22, 1, 22, 0, 0), (28, 1, 16, 0, 0)],
    // v3
    [(15, 1, 55, 0, 0), (26, 1, 44, 0, 0), (18, 2, 17, 0, 0), (22, 2, 13, 0, 0)],
    // v4
    [(20, 1, 80, 0, 0), (18, 2, 32, 0, 0), (26, 2, 24, 0, 0), (16, 4, 9, 0, 0)],
    // v5
    [(26, 1, 108, 0, 0), (24, 2, 43, 0, 0), (18, 2, 15, 2, 16), (22, 2, 11, 2, 12)],
    // v6
    [(18, 2, 68, 0, 0), (16, 4, 27, 0, 0), (24, 4, 19, 0, 0), (28, 4, 15, 0, 0)],
    // v7
    [(20, 2, 78, 0, 0), (18, 4, 31, 0, 0), (18, 2, 14, 4, 15), (26, 4, 13, 1, 14)],
    // v8
    [(24, 2, 97, 0, 0), (22, 2, 38, 2, 39), (22, 4, 18, 2, 19), (26, 4, 14, 2, 15)],
    // v9
    [(30, 2, 116, 0, 0), (22, 3, 36, 2, 37), (20, 4, 16, 4, 17), (24, 4, 12, 4, 13)],
    // v10
    [(18, 2, 68, 2, 69), (26, 4, 43, 1, 44), (24, 6, 19, 2, 20), (28, 6, 15, 2, 16)],
    // v11
    [(20, 4, 81, 0, 0), (30, 1, 50, 4, 51), (28, 4, 22, 4, 23), (24, 3, 12, 8, 13)],
    // v12
    [(24, 2, 92, 2, 93), (22, 6, 36, 2, 37), (26, 4, 20, 6, 21), (28, 7, 14, 4, 15)],
    // v13
    [(26, 4, 107, 0, 0), (22, 8, 37, 1, 38), (24, 8, 20, 4, 21), (22, 12, 11, 4, 12)],
    // v14
    [(30, 3, 115, 1, 116), (24, 4, 40, 5, 41), (20, 11, 16, 5, 17), (24, 11, 12, 5, 13)],
    // v15
    [(22, 5, 87, 1, 88), (24, 5, 41, 5, 42), (30, 5, 24, 7, 25), (24, 11, 12, 7, 13)],
    // v16
    [(24, 5, 98, 1, 99), (28, 7, 45, 3, 46), (24, 15, 19, 2, 20), (30, 3, 15, 13, 16)],
    // v17
    [(28, 1, 107, 5, 108), (28, 10, 46, 1, 47), (28, 1, 22, 15, 23), (28, 2, 14, 17, 15)],
    // v18
    [(30, 5, 120, 1, 121), (26, 9, 43, 4, 44), (28, 17, 22, 1, 23), (28, 2, 14, 19, 15)],
    // v19
    [(28, 3, 113, 4, 114), (26, 3, 44, 11, 45), (26, 17, 21, 4, 22), (26, 9, 13, 16, 14)],
    // v20
    [(28, 3, 107, 5, 108), (26, 3, 41, 13, 42), (30, 15, 24, 5, 25), (28, 15, 15, 10, 16)],
    // v21
    [(28, 4, 116, 4, 117), (26, 17, 42, 0, 0), (28, 17, 22, 6, 23), (30, 19, 16, 6, 17)],
    // v22
    [(28, 2, 111, 7, 112), (28, 17, 46, 0, 0), (30, 7, 24, 16, 25), (24, 34, 13, 0, 0)],
    // v23
    [(30, 4, 121, 5, 122), (28, 4, 47, 14, 48), (30, 11, 24, 14, 25), (30, 16, 15, 14, 16)],
    // v24
    [(30, 6, 117, 4, 118), (28, 6, 45, 14, 46), (30, 11, 24, 16, 25), (30, 30, 16, 2, 17)],
    // v25
    [(26, 8, 106, 4, 107), (28, 8, 47, 13, 48), (30, 7, 24, 22, 25), (30, 22, 15, 13, 16)],
    // v26
    [(28, 10, 114, 2, 115), (28, 19, 46, 4, 47), (28, 28, 22, 6, 23), (30, 33, 16, 4, 17)],
    // v27
    [(30, 8, 122, 4, 123), (28, 22, 45, 3, 46), (30, 8, 23, 26, 24), (30, 12, 15, 28, 16)],
    // v28
    [(30, 3, 117, 10, 118), (28, 3, 45, 23, 46), (30, 4, 24, 31, 25), (30, 11, 15, 31, 16)],
    // v29
    [(30, 7, 116, 7, 117), (28, 21, 45, 7, 46), (30, 1, 23, 37, 24), (30, 19, 15, 26, 16)],
    // v30
    [(30, 5, 115, 10, 116), (28, 19, 47, 10, 48), (30, 15, 24, 25, 25), (30, 23, 15, 25, 16)],
    // v31
    [(30, 13, 115, 3, 116), (28, 2, 46, 29, 47), (30, 42, 24, 1, 25), (30, 23, 15, 28, 16)],
    // v32
    [(30, 17, 115, 0, 0), (28, 10, 46, 23, 47), (30, 10, 24, 35, 25), (30, 19, 15, 35, 16)],
    // v33
    [(30, 17, 115, 1, 116), (28, 14, 46, 21, 47), (30, 29, 24, 19, 25), (30, 11, 15, 46, 16)],
    // v34
    [(30, 13, 115, 6, 116), (28, 14, 46, 23, 47), (30, 44, 24, 7, 25), (30, 59, 16, 1, 17)],
    // v35
    [(30, 12, 121, 7, 122), (28, 12, 47, 26, 48), (30, 39, 24, 14, 25), (30, 22, 15, 41, 16)],
    // v36
    [(30, 6, 121, 14, 122), (28, 6, 47, 34, 48), (30, 46, 24, 10, 25), (30, 2, 15, 64, 16)],
    // v37
    [(30, 17, 122, 4, 123), (28, 29, 46, 14, 47), (30, 49, 24, 10, 25), (30, 24, 15, 46, 16)],
    // v38
    [(30, 4, 122, 18, 123), (28, 13, 46, 32, 47), (30, 48, 24, 14, 25), (30, 42, 15, 32, 16)],
    // v39
    [(30, 20, 117, 4, 118), (28, 40, 47, 7, 48), (30, 43, 24, 22, 25), (30, 10, 15, 67, 16)],
    // v40
    [(30, 19, 118, 6, 119), (28, 18, 47, 31, 48), (30, 34, 24, 34, 25), (30, 20, 15, 61, 16)],
];
