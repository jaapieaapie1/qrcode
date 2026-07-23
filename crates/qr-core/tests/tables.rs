//! Version / EC-block table invariants across all 40 versions x 4 levels, and a
//! geometric cross-check that the function-pattern map agrees with the codeword
//! counts.

use qr_core::{ec_blocks, is_function_module, EcLevel, Version};

#[test]
fn block_sums_match_total_codewords() {
    for v in 1..=40u8 {
        let version = Version::new(v).unwrap();
        for ec in EcLevel::ALL {
            let b = ec_blocks(version, ec);
            assert_eq!(
                b.total_codewords(),
                version.total_codewords(),
                "v{v} {ec:?}: data+ec ({}) != total codewords ({})",
                b.total_codewords(),
                version.total_codewords(),
            );
            // Group data lengths are contiguous: group2 blocks hold exactly one
            // more data codeword than group1 blocks (or group2 is empty).
            if b.group2_blocks > 0 {
                assert_eq!(
                    b.group2_data,
                    b.group1_data + 1,
                    "v{v} {ec:?}: group2 data not group1+1"
                );
            }
        }
    }
}

/// Independently counts non-function (data-region) modules from the geometric
/// function-pattern map and checks it equals `8 * total_codewords + remainder`.
/// This links the geometry, the total-codeword table and the block table.
#[test]
fn function_map_matches_data_region_size() {
    for v in 1..=40u8 {
        let version = Version::new(v).unwrap();
        let size = version.size();
        let mut data_modules = 0u32;
        for y in 0..size {
            for x in 0..size {
                if !is_function_module(version, x, y) {
                    data_modules += 1;
                }
            }
        }
        let expected = 8 * version.total_codewords() as u32 + version.remainder_bits() as u32;
        assert_eq!(
            data_modules, expected,
            "v{v}: data-region modules {data_modules} != 8*codewords+remainder {expected}"
        );
    }
}

#[test]
fn size_formula() {
    assert_eq!(Version::new(1).unwrap().size(), 21);
    assert_eq!(Version::new(40).unwrap().size(), 177);
}

#[test]
fn version_range_rejects_out_of_bounds() {
    assert!(Version::new(0).is_err());
    assert!(Version::new(41).is_err());
    assert!(Version::new(1).is_ok());
    assert!(Version::new(40).is_ok());
}
