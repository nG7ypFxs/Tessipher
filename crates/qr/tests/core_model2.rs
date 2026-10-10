use qr::core::*;
fn decode_bits(bits: &[u8], side: usize) -> Result<(Decoded, Vec<u8>), Error> {
    let mut scratch = vec![0; SCRATCH_BYTES];
    let mut out = vec![0; MAX_PAYLOAD_BYTES];
    let mut segments = vec![Segment::EMPTY; MAX_SEGMENTS];
    let d = decode(
        Matrix::new(bits, side)?,
        &mut scratch,
        &mut out,
        &mut segments,
    )?;
    out.truncate(d.payload_len);
    Ok((d, out))
}
#[test]
fn independently_generated_all_1280_normal_and_transposed() {
    let corpus = include_bytes!("../../../fixtures/all-model2.bin");
    assert_eq!(&corpus[..4], b"TQR1");
    let mut p = 4;
    let mut count = 0;
    while p < corpus.len() {
        let v = corpus[p];
        let e = corpus[p + 1];
        let mask = corpus[p + 2];
        let len = u16::from_le_bytes([corpus[p + 3], corpus[p + 4]]) as usize;
        p += 5;
        let side = v as usize * 4 + 17;
        let mirrored = transpose(&corpus[p..p + len], side);
        for bits in [&corpus[p..p + len], &mirrored] {
            let (d, out) = decode_bits(bits, side)
                .unwrap_or_else(|err| panic!("v={v} ec={e} mask={mask}: {err:?}"));
            assert_eq!((d.version, d.ec_level as u8, d.mask), (v, e, mask));
            assert_eq!(
                out,
                format!("{v:02}{}{mask}", b"LMQH"[e as usize] as char).as_bytes()
            );
        }
        p += len;
        count += 1;
    }
    assert_eq!(count, 1280);
}
fn transpose(bits: &[u8], side: usize) -> Vec<u8> {
    let mut result = vec![0; bits.len()];
    let matrix = Matrix::new(bits, side).unwrap();
    for y in 0..side {
        for x in 0..side {
            if matrix.get(y, x) == Some(true) {
                flip(&mut result, side, x, y);
            }
        }
    }
    result
}
fn corpus_matrix(version: u8, ecc: u8, mask: u8) -> &'static [u8] {
    let corpus = include_bytes!("../../../fixtures/all-model2.bin");
    let mut p = 4;
    while p < corpus.len() {
        let len = u16::from_le_bytes([corpus[p + 3], corpus[p + 4]]) as usize;
        if corpus[p..p + 3] == [version, ecc, mask] {
            return &corpus[p + 5..p + 5 + len];
        }
        p += 5 + len;
    }
    panic!("missing corpus matrix");
}
#[test]
fn rs_recommended_reserve_boundaries() {
    // Single-block cases include every nonzero reserve and unchanged limits.
    for (version, ecc, limit) in [
        (1, 0, 2),
        (1, 1, 4),
        (2, 0, 4),
        (1, 2, 6),
        (1, 3, 8),
        (3, 0, 7),
        (2, 1, 8),
    ] {
        let original = corpus_matrix(version, ecc, 0);
        let coords = data_coords(version);
        let side = version as usize * 4 + 17;
        for errors in 0..=limit + 1 {
            let mut bits = original.to_vec();
            for i in 0..errors {
                let (x, y) = coords[i * 8];
                flip(&mut bits, side, x, y);
            }
            let result = decode_bits(&bits, side);
            if errors <= limit {
                let (decoded, out) = result.unwrap_or_else(|error| {
                    panic!("v{version} ecc{ecc} errors{errors}: {error:?}")
                });
                assert_eq!(
                    out,
                    format!("{version:02}{}0", b"LMQH"[ecc as usize] as char).as_bytes()
                );
                assert_eq!(decoded.corrected_codewords as usize, errors);
            } else {
                assert_eq!(result, Err(Error::ReedSolomon), "v{version} ecc{ecc}");
            }
        }
    }
}
fn flip(bits: &mut [u8], side: usize, x: usize, y: usize) {
    let i = y * side + x;
    bits[i / 8] ^= 128 >> (i % 8);
}
fn data_coords(v: u8) -> Vec<(usize, usize)> {
    let n = v as usize * 4 + 17;
    let mut coords = vec![];
    let mut r = n - 1;
    let mut up = true;
    while r > 0 {
        if r == 6 {
            r -= 1;
        }
        for row in 0..n {
            let y = if up { n - 1 - row } else { row };
            for x in [r, r - 1] {
                if !is_function(v, x, y) {
                    coords.push((x, y));
                }
            }
        }
        r = r.saturating_sub(2);
        up = !up;
    }
    coords
}
#[test]
fn rs_corrects_per_block_capacity_and_rejects_known_excess() {
    let original = include_bytes!("../../../fixtures/v1.bin");
    let coords = data_coords(1);
    for errors in 0..=8 {
        let mut bits = original.to_vec();
        for i in 0..errors {
            let (x, y) = coords[i * 8];
            flip(&mut bits, 21, x, y);
        }
        let (d, out) = decode_bits(&bits, 21).unwrap();
        assert_eq!(out, b"TEST");
        assert_eq!(d.corrected_codewords as usize, errors);
    }
    let mut bits = original.to_vec();
    for i in 0..12 {
        for k in 0..8 {
            let (x, y) = coords[i * 8 + k];
            flip(&mut bits, 21, x, y);
        }
    }
    assert_eq!(decode_bits(&bits, 21), Err(Error::ReedSolomon));
}
#[test]
fn bch_three_errors_and_conflicting_copies() {
    let original = include_bytes!("../../../fixtures/v7.bin");
    let n = 45;
    let mut bits = original.to_vec();
    for i in 0..3 {
        flip(&mut bits, n, 8, i);
        flip(&mut bits, n, n - 1 - i, 8);
        flip(&mut bits, n, n - 11 + i, 0);
        flip(&mut bits, n, 0, n - 11 + i);
    }
    let (d, _) = decode_bits(&bits, n).unwrap();
    assert_eq!(d.format_distance, 3);
    assert_eq!(d.version_distance, 3);
    // Copy valid second-format data from a different independent mask into one copy.
    let corpus = include_bytes!("../../../fixtures/all-model2.bin");
    let mut p = 4;
    let mut other = &corpus[0..0];
    while p < corpus.len() {
        let v = corpus[p];
        let e = corpus[p + 1];
        let m = corpus[p + 2];
        let l = u16::from_le_bytes([corpus[p + 3], corpus[p + 4]]) as usize;
        p += 5;
        if (v, e, m) == (7, 3, 4) {
            other = &corpus[p..p + l];
            break;
        }
        p += l;
    }
    let mut bits = original.to_vec();
    for i in 0..15 {
        let (x, y) = if i < 8 {
            (n - 1 - i, 8)
        } else {
            (8, n - 15 + i)
        };
        let pos = y * n + x;
        bits[pos / 8] =
            (bits[pos / 8] & !(128 >> (pos % 8))) | (other[pos / 8] & (128 >> (pos % 8)));
    }
    assert_eq!(decode_bits(&bits, n), Err(Error::Format));
}
#[test]
fn invalid_dimensions_and_capacities() {
    assert!(Matrix::new(&[], usize::MAX).is_err());
    assert!(Matrix::new(&[], 21).is_err());
    let bits = include_bytes!("../../../fixtures/v1.bin");
    let mirrored = transpose(bits, 21);
    for bits in [bits.as_slice(), &mirrored] {
        let m = Matrix::new(bits, 21).unwrap();
        let mut scratch = vec![0; SCRATCH_BYTES];
        let mut out = [0; 4];
        let mut seg = [Segment::EMPTY; 1];
        assert_eq!(
            decode(m, &mut [], &mut out, &mut seg),
            Err(Error::ScratchTooSmall)
        );
        assert_eq!(
            decode(m, &mut scratch, &mut out[..3], &mut seg),
            Err(Error::OutputTooSmall)
        );
        assert_eq!(
            decode(m, &mut scratch, &mut out, &mut []),
            Err(Error::SegmentsTooSmall)
        );
    }
}
#[test]
fn maximum_payloads() {
    for (name, bits) in [
        (
            "numeric",
            include_bytes!("../../../fixtures/numeric-max.bin").as_slice(),
        ),
        (
            "byte",
            include_bytes!("../../../fixtures/byte-max.bin").as_slice(),
        ),
        (
            "kanji",
            include_bytes!("../../../fixtures/kanji-max.bin").as_slice(),
        ),
    ] {
        let (d, out) = decode_bits(bits, 177).unwrap();
        assert_eq!(d.version, 40);
        match name {
            "numeric" => assert_eq!(out, vec![b'1'; 7089]),
            "byte" => assert_eq!(out, (0..2953).map(|i| i as u8).collect::<Vec<_>>()),
            _ => assert_eq!(out, [0x8a, 0xbf].repeat(1817)),
        }
    }
}
#[test]
fn independent_mixed_modes_and_control_metadata() {
    let bits = include_bytes!("../../../fixtures/modes.bin");
    let mut scratch = vec![0; SCRATCH_BYTES];
    let mut out = [0; 128];
    let mut seg = [Segment::EMPTY; 16];
    let d = decode(
        Matrix::new(bits, 37).unwrap(),
        &mut scratch,
        &mut out,
        &mut seg,
    )
    .unwrap();
    assert_eq!(
        &out[..d.payload_len],
        b"0012345A\x1dB%C\x00\xff%\x8a\xbf\x8e\x9a"
    );
    assert_eq!(d.fnc1, Fnc1::Second(42));
    assert_eq!(
        d.structured_append,
        Some(StructuredAppend {
            index: 0,
            total: 2,
            parity: 0xa7
        })
    );
    assert_eq!(d.segment_count, 7);
    assert_eq!(seg[2].mode, Mode::Eci);
    assert!(seg[2..7].iter().all(|s| s.eci == 26));
    assert_eq!(seg[6].mode, Mode::Kanji);
}
#[test]
fn interleaved_unequal_blocks_correct_random_errors() {
    // V7-H: 4 blocks of 13 data + 26 ECC, 1 block of 14 data + 26 ECC.
    let original = include_bytes!("../../../fixtures/v7.bin");
    let coords = data_coords(7);
    let mut bits = original.to_vec();
    let mut state = 0x12345678u32;
    for column in 0..13 {
        for block in 0..5 {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let delta = (state as u8) | 1;
            let offset = (column * 5 + block) * 8;
            for k in 0..8 {
                if delta & (128 >> k) != 0 {
                    let (x, y) = coords[offset + k];
                    flip(&mut bits, 45, x, y);
                }
            }
        }
    }
    let (d, out) = decode_bits(&bits, 45).unwrap();
    assert_eq!(out, b"TEST");
    assert_eq!(d.corrected_codewords, 65);
}

#[test]
fn second_independent_generator_segno_mode_and_version_boundaries() {
    let corpus = include_bytes!("../../../fixtures/segno-boundaries.bin");
    assert_eq!(&corpus[..4], b"TQS1");
    let mut cursor = 4;
    let mut count = 0;
    let mut scratch = vec![0; SCRATCH_BYTES];
    let mut output = vec![0; MAX_PAYLOAD_BYTES];
    let mut segments = [Segment::EMPTY; 8];
    while cursor < corpus.len() {
        let version = corpus[cursor];
        let ecc = corpus[cursor + 1];
        let mask = corpus[cursor + 2];
        let mode = corpus[cursor + 3];
        let payload_len = u16::from_le_bytes([corpus[cursor + 4], corpus[cursor + 5]]) as usize;
        let matrix_len = u16::from_le_bytes([corpus[cursor + 6], corpus[cursor + 7]]) as usize;
        cursor += 8;
        let expected = &corpus[cursor..cursor + payload_len];
        cursor += payload_len;
        let matrix = Matrix::new(
            &corpus[cursor..cursor + matrix_len],
            version as usize * 4 + 17,
        )
        .unwrap();
        let decoded =
            decode(matrix, &mut scratch, &mut output, &mut segments).unwrap_or_else(|error| {
                panic!("Segno v{version} ecc{ecc} mask{mask} mode{mode}: {error:?}")
            });
        assert_eq!(
            (decoded.version, decoded.ec_level as u8, decoded.mask),
            (version, ecc, mask)
        );
        assert_eq!(&output[..decoded.payload_len], expected);
        assert_eq!(decoded.segment_count, 1);
        assert_eq!(segments[0].mode as u8, mode);
        cursor += matrix_len;
        count += 1;
    }
    assert_eq!(count, 24);
}

#[test]
fn independent_eci_before_fnc_headers() {
    for (bits, second, append) in [
        (
            include_bytes!("../../../fixtures/eci-fnc1.bin").as_slice(),
            false,
            false,
        ),
        (
            include_bytes!("../../../fixtures/eci-fnc2.bin").as_slice(),
            true,
            false,
        ),
        (
            include_bytes!("../../../fixtures/eci-fnc1-sa.bin").as_slice(),
            false,
            true,
        ),
        (
            include_bytes!("../../../fixtures/eci-fnc2-sa.bin").as_slice(),
            true,
            true,
        ),
    ] {
        let mut scratch = vec![0; SCRATCH_BYTES];
        let mut out = [0; 32];
        let mut segments = [Segment::EMPTY; 8];
        let decoded = decode(
            Matrix::new(bits, 21).unwrap(),
            &mut scratch,
            &mut out,
            &mut segments,
        )
        .unwrap();
        assert_eq!(&out[..decoded.payload_len], b"A\x1dB%C%\xff");
        assert_eq!(
            decoded.fnc1,
            if second {
                Fnc1::Second(42)
            } else {
                Fnc1::First
            }
        );
        assert_eq!(
            decoded.structured_append,
            append.then_some(StructuredAppend {
                index: 0,
                total: 2,
                parity: 0xa7
            })
        );
        let mut modes = vec![];
        if append {
            modes.push(Mode::StructuredAppend);
        }
        modes.extend([
            Mode::Eci,
            if second {
                Mode::Fnc1Second
            } else {
                Mode::Fnc1First
            },
            Mode::Alphanumeric,
            Mode::Byte,
        ]);
        assert_eq!(decoded.segment_count, modes.len());
        assert_eq!(
            segments[..decoded.segment_count]
                .iter()
                .map(|s| s.mode)
                .collect::<Vec<_>>(),
            modes
        );
        assert!(segments[usize::from(append)..decoded.segment_count]
            .iter()
            .all(|s| s.eci == 26));
        if append {
            assert_eq!(segments[0].eci, 3);
        }
    }
}

#[test]
fn independent_fnc_percent_pair_across_segments_normal_and_transposed() {
    for (bits, second, split) in [
        (
            include_bytes!("../../../fixtures/fnc1-percent-joined.bin").as_slice(),
            false,
            false,
        ),
        (
            include_bytes!("../../../fixtures/fnc1-percent-split.bin").as_slice(),
            false,
            true,
        ),
        (
            include_bytes!("../../../fixtures/fnc2-percent-joined.bin").as_slice(),
            true,
            false,
        ),
        (
            include_bytes!("../../../fixtures/fnc2-percent-split.bin").as_slice(),
            true,
            true,
        ),
    ] {
        let mirrored = transpose(bits, 21);
        for matrix_bits in [bits, mirrored.as_slice()] {
            let mut scratch = vec![0; SCRATCH_BYTES];
            let mut out = [0; 1];
            let mut segments = [Segment::EMPTY; 3];
            let decoded = decode(
                Matrix::new(matrix_bits, 21).unwrap(),
                &mut scratch,
                &mut out,
                &mut segments,
            )
            .unwrap();
            assert_eq!(out, [b'%']);
            assert_eq!(
                (decoded.version, decoded.ec_level, decoded.mask),
                (1, EcLevel::H, 0)
            );
            assert_eq!(decoded.payload_len, 1);
            assert_eq!(decoded.segment_count, if split { 3 } else { 2 });
            assert_eq!(decoded.corrected_codewords, 0);
            assert_eq!(
                decoded.fnc1,
                if second {
                    Fnc1::Second(42)
                } else {
                    Fnc1::First
                }
            );
            assert_eq!(decoded.structured_append, None);
            assert_eq!(
                (segments[0].mode, segments[0].offset, segments[0].len),
                (
                    if second {
                        Mode::Fnc1Second
                    } else {
                        Mode::Fnc1First
                    },
                    0,
                    0
                )
            );
            assert_eq!(
                (segments[1].mode, segments[1].offset, segments[1].len),
                (Mode::Alphanumeric, 0, 1)
            );
            if split {
                assert_eq!(
                    (segments[2].mode, segments[2].offset, segments[2].len),
                    (Mode::Alphanumeric, 1, 0)
                );
            }
            assert!(segments[..decoded.segment_count]
                .iter()
                .all(|segment| segment.eci == 3));
        }
    }
}
