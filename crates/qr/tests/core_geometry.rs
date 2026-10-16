use tessipher::core::{alignment_positions, is_function, raw_codewords};

// The pre-cache pairwise reference deliberately visits every alignment center pair.
// This catches errors at finder exclusions, timing intersections and band boundaries.
fn pairwise_reference(version: u8, x: usize, y: usize) -> bool {
    let side = version as usize * 4 + 17;
    if (x < 9 && (y < 9 || y >= side - 8))
        || (y < 9 && x >= side - 8)
        || x == 6
        || y == 6
        || (version >= 7 && ((y < 6 && x >= side - 11) || (x < 6 && y >= side - 11)))
    {
        return true;
    }
    let mut positions = [0; 7];
    let count = alignment_positions(version, &mut positions);
    for i in 0..count {
        for j in 0..count {
            if (i == 0 && (j == 0 || j == count - 1)) || (j == 0 && i == count - 1) {
                continue;
            }
            if x.abs_diff(positions[i] as usize) <= 2 && y.abs_diff(positions[j] as usize) <= 2 {
                return true;
            }
        }
    }
    false
}

#[test]
fn all_versions_all_coordinates_match_pairwise_geometry() {
    for version in 1..=40 {
        let side = version as usize * 4 + 17;
        let mut data_modules = 0;
        for y in 0..side {
            for x in 0..side {
                let actual = is_function(version, x, y);
                assert_eq!(
                    actual,
                    pairwise_reference(version, x, y),
                    "v{version} ({x},{y})"
                );
                data_modules += usize::from(!actual);
            }
        }
        assert_eq!(data_modules / 8, raw_codewords(version), "v{version}");
        for coordinate in [side, side + 1, usize::MAX] {
            assert!(is_function(version, coordinate, 0));
            assert!(is_function(version, 0, coordinate));
        }
    }
}

#[test]
fn alignment_special_spacing_and_invalid_inputs() {
    let mut positions = [255; 7];
    assert_eq!(alignment_positions(32, &mut positions), 6);
    assert_eq!(positions, [6, 34, 60, 86, 112, 138, 255]);
    for version in [0, 1, 41, 255] {
        let mut positions = [255; 7];
        assert_eq!(alignment_positions(version, &mut positions), 0);
        assert_eq!(positions, [255; 7]);
    }
    for version in [0, 41, 255] {
        assert_eq!(raw_codewords(version), 0);
        for (x, y) in [(0, 0), (6, 6), (177, 177), (usize::MAX, usize::MAX)] {
            assert!(is_function(version, x, y));
        }
    }
}
