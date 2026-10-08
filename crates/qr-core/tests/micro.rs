#![cfg(feature = "micro-qr")]

use qr_core::{
    decode_micro, Error, MicroDecoded, MicroMatrix, MicroVersion, Segment, MICRO_MAX_PAYLOAD_BYTES,
    MICRO_SCRATCH_BYTES,
};

fn decode_bits(bits: &[u8], side: usize) -> Result<(MicroDecoded, Vec<u8>), Error> {
    let mut scratch = vec![0; MICRO_SCRATCH_BYTES];
    let mut output = vec![0; MICRO_MAX_PAYLOAD_BYTES];
    let mut segments = [Segment::EMPTY; 8];
    let decoded = decode_micro(
        MicroMatrix::new(bits, side)?,
        &mut scratch,
        &mut output,
        &mut segments,
    )?;
    output.truncate(decoded.payload_len);
    Ok((decoded, output))
}

fn transform(bits: &[u8], side: usize, transform: u8) -> Vec<u8> {
    let matrix = MicroMatrix::new(bits, side).unwrap();
    let mut result = vec![0; bits.len()];
    let n = side - 1;
    for y in 0..side {
        for x in 0..side {
            let (sx, sy) = match transform {
                0 => (x, y),
                1 => (n - y, x),
                2 => (n - x, n - y),
                3 => (y, n - x),
                4 => (y, x),
                5 => (n - x, y),
                6 => (n - y, n - x),
                _ => (x, n - y),
            };
            if matrix.get(sx, sy) == Some(true) {
                let i = y * side + x;
                result[i / 8] |= 0x80 >> (i % 8);
            }
        }
    }
    result
}

#[test]
fn independent_all_versions_ecc_modes_masks_and_orientations() {
    let corpus = include_bytes!("../../../fixtures/micro-segno.bin");
    assert_eq!(&corpus[..4], b"TQM1");
    let mut cursor = 4;
    let mut count = 0;
    while cursor < corpus.len() {
        let version = corpus[cursor];
        let ecc = corpus[cursor + 1];
        let mask = corpus[cursor + 2];
        let mode = corpus[cursor + 3];
        let output_len = u16::from_le_bytes([corpus[cursor + 4], corpus[cursor + 5]]) as usize;
        let matrix_len = u16::from_le_bytes([corpus[cursor + 6], corpus[cursor + 7]]) as usize;
        cursor += 8;
        let expected = &corpus[cursor..cursor + output_len];
        cursor += output_len;
        let original = &corpus[cursor..cursor + matrix_len];
        cursor += matrix_len;
        let side = version as usize * 2 + 9;
        for orientation in 0..8 {
            let bits = transform(original, side, orientation);
            let (decoded, output) = decode_bits(&bits, side).unwrap_or_else(|error| {
                panic!(
                    "M{version} ecc={ecc} mask={mask} mode={mode} orientation={orientation}: {error:?}"
                )
            });
            assert_eq!(decoded.version as u8, version);
            assert_eq!(
                decoded.ec_level.map(|level| level as u8).unwrap_or(255),
                ecc
            );
            assert_eq!(decoded.mask, mask);
            assert_eq!(output, expected);
        }
        count += 1;
    }
    assert_eq!(count, 125);
}

#[test]
fn dimensions_and_caller_capacities_are_checked() {
    assert!(matches!(
        MicroMatrix::new(&[], 11),
        Err(Error::InvalidMatrix)
    ));
    assert!(matches!(
        MicroMatrix::new(&[0; 38], 12),
        Err(Error::InvalidMatrix)
    ));
    let corpus = include_bytes!("../../../fixtures/micro-segno.bin");
    let output_len = u16::from_le_bytes([corpus[8], corpus[9]]) as usize;
    let matrix_len = u16::from_le_bytes([corpus[10], corpus[11]]) as usize;
    let bits = &corpus[12 + output_len..12 + output_len + matrix_len];
    let matrix = MicroMatrix::new(bits, 11).unwrap();
    assert_eq!(matrix.version(), MicroVersion::M1);
    let mut scratch = [0; MICRO_SCRATCH_BYTES];
    let mut output = [0; 3];
    let mut segments = [Segment::EMPTY; 1];
    assert_eq!(
        decode_micro(matrix, &mut [], &mut output, &mut segments),
        Err(Error::ScratchTooSmall)
    );
    assert_eq!(
        decode_micro(matrix, &mut scratch, &mut output[..2], &mut segments),
        Err(Error::OutputTooSmall)
    );
    assert_eq!(
        decode_micro(matrix, &mut scratch, &mut output, &mut []),
        Err(Error::SegmentsTooSmall)
    );
}

fn fixture(version: u8, ecc: u8) -> &'static [u8] {
    let corpus = include_bytes!("../../../fixtures/micro-segno.bin");
    let mut cursor = 4;
    while cursor < corpus.len() {
        let current_version = corpus[cursor];
        let current_ecc = corpus[cursor + 1];
        let output_len = u16::from_le_bytes([corpus[cursor + 4], corpus[cursor + 5]]) as usize;
        let matrix_len = u16::from_le_bytes([corpus[cursor + 6], corpus[cursor + 7]]) as usize;
        cursor += 8 + output_len;
        let bits = &corpus[cursor..cursor + matrix_len];
        cursor += matrix_len;
        if (current_version, current_ecc) == (version, ecc) {
            return bits;
        }
    }
    panic!("missing M{version} ECC {ecc}")
}

fn data_coordinates(version: u8) -> Vec<(usize, usize)> {
    let side = version as usize * 2 + 9;
    let inc = if version == 1 || version == 3 { 2 } else { 0 };
    let mut coordinates = vec![];
    let mut right = side - 1;
    while right > 0 {
        let upwards = (right + inc) & 2 == 0;
        for vertical in 0..side {
            let y = if upwards {
                side - 1 - vertical
            } else {
                vertical
            };
            for x in [right, right - 1] {
                if !((x < 9 && y < 9) || x == 0 || y == 0) {
                    coordinates.push((x, y));
                }
            }
        }
        right -= 2;
    }
    coordinates
}

fn flip(bits: &mut [u8], side: usize, coordinate: (usize, usize)) {
    let bit = coordinate.1 * side + coordinate.0;
    bits[bit / 8] ^= 0x80 >> (bit % 8);
}

#[test]
fn reed_solomon_boundaries_and_m1_detection_only() {
    let m1 = fixture(1, 255);
    let mut damaged = m1.to_vec();
    flip(&mut damaged, 11, data_coordinates(1)[0]);
    assert_eq!(decode_bits(&damaged, 11), Err(Error::ReedSolomon));

    for (version, ecc, correction_limit) in [
        (2, 0, 1),
        (2, 1, 2),
        (3, 0, 2),
        (3, 1, 4),
        (4, 0, 3),
        (4, 1, 5),
        (4, 2, 7),
    ] {
        let original = fixture(version, ecc);
        let side = version as usize * 2 + 9;
        let coordinates = data_coordinates(version);
        for errors in 0..=correction_limit + 1 {
            let mut bits = original.to_vec();
            for index in 0..errors {
                flip(&mut bits, side, coordinates[index * 8]);
            }
            let result = decode_bits(&bits, side);
            if errors <= correction_limit {
                let (decoded, _) = result.unwrap_or_else(|error| {
                    panic!("M{version} ECC {ecc}, {errors} errors: {error:?}")
                });
                assert_eq!(decoded.corrected_codewords as usize, errors);
            } else {
                assert_eq!(result, Err(Error::ReedSolomon));
            }
        }
    }
}

#[test]
fn format_bch_corrects_three_bits_and_rejects_known_four_bit_damage() {
    let original = fixture(4, 2);
    let mut damaged = original.to_vec();
    for y in 1..=3 {
        flip(&mut damaged, 17, (8, y));
    }
    let (decoded, _) = decode_bits(&damaged, 17).unwrap();
    assert_eq!(decoded.format_distance, 3);

    let mut found_rejection = false;
    let format_coordinates: Vec<_> = (1..=7)
        .map(|y| (8, y))
        .chain((1..=8).map(|x| (x, 8)))
        .collect();
    'outer: for a in 0..format_coordinates.len() {
        for b in a + 1..format_coordinates.len() {
            for c in b + 1..format_coordinates.len() {
                for d in c + 1..format_coordinates.len() {
                    let mut bits = original.to_vec();
                    for index in [a, b, c, d] {
                        flip(&mut bits, 17, format_coordinates[index]);
                    }
                    if decode_bits(&bits, 17) == Err(Error::Format) {
                        found_rejection = true;
                        break 'outer;
                    }
                }
            }
        }
    }
    assert!(found_rejection);
}

#[test]
fn fixed_finder_separator_and_timing_modules_are_validated() {
    let original = fixture(4, 2);
    for coordinate in [(0, 0), (1, 1), (7, 4), (0, 10), (12, 0)] {
        let mut damaged = original.to_vec();
        flip(&mut damaged, 17, coordinate);
        assert_eq!(decode_bits(&damaged, 17), Err(Error::InvalidMatrix));
    }
}
