use qr::core::*;
#[derive(Default)]
struct Bits(Vec<bool>);
impl Bits {
    fn add(mut self, v: u32, n: usize) -> Self {
        for i in (0..n).rev() {
            self.0.push(v & (1 << i) != 0);
        }
        self
    }
    fn bytes(self) -> Vec<u8> {
        let mut out = vec![0; self.0.len().div_ceil(8)];
        for (i, b) in self.0.into_iter().enumerate() {
            if b {
                out[i / 8] |= 128 >> (i % 8);
            }
        }
        out
    }
    fn fnc(self, second: bool) -> Self {
        if second {
            self.add(9, 4).add(42, 8)
        } else {
            self.add(5, 4)
        }
    }
    fn alphanumeric(mut self, text: &[u8]) -> Self {
        const ALPHA: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";
        self = self.add(2, 4).add(text.len() as u32, 9);
        for pair in text.chunks(2) {
            let first = ALPHA.iter().position(|&ch| ch == pair[0]).unwrap() as u32;
            self = if pair.len() == 2 {
                let second = ALPHA.iter().position(|&ch| ch == pair[1]).unwrap() as u32;
                self.add(first * 45 + second, 11)
            } else {
                self.add(first, 6)
            };
        }
        self
    }
}
type Parsed = (Vec<u8>, Vec<Segment>, Fnc1, Option<StructuredAppend>);
fn parse(b: Bits) -> Result<Parsed, Error> {
    let mut out = vec![0; 7089];
    let mut seg = vec![Segment::EMPTY; 128];
    let data = b.bytes();
    let mut legacy_out = out.clone();
    let mut legacy_seg = seg.clone();
    let named = parse_payload_info(&data, 1, &mut out, &mut seg);
    let legacy = parse_payload(&data, 1, &mut legacy_out, &mut legacy_seg);
    assert_eq!(
        named.map(|r| (r.payload_len, r.segment_count, r.fnc1, r.structured_append)),
        legacy
    );
    let info = named?;
    out.truncate(info.payload_len);
    seg.truncate(info.segment_count);
    assert_eq!(out, legacy_out[..info.payload_len]);
    assert_eq!(seg, legacy_seg[..info.segment_count]);
    Ok((out, seg, info.fnc1, info.structured_append))
}

#[test]
fn named_payload_metadata_and_boundaries() {
    for version in [1, 9, 10, 26, 27, 40] {
        let data = Bits::default()
            .add(4, 4)
            .add(1, if version <= 9 { 8 } else { 16 })
            .add(0xff, 8)
            .add(0, 4)
            .bytes();
        let mut output = [0; 1];
        let mut segments = [Segment::EMPTY; 1];
        assert_eq!(
            parse_payload_info(&data, version, &mut output, &mut segments),
            Ok(PayloadInfo {
                payload_len: 1,
                segment_count: 1,
                fnc1: Fnc1::None,
                structured_append: None,
            })
        );
        assert_eq!(output, [0xff]);
        assert_eq!(segments[0].mode, Mode::Byte);
        assert_eq!(
            (segments[0].offset, segments[0].len, segments[0].eci),
            (0, 1, 3)
        );
        for (out_capacity, segment_capacity, error) in [
            (0, 1, Error::OutputTooSmall),
            (1, 0, Error::SegmentsTooSmall),
        ] {
            assert_eq!(
                parse_payload_info(
                    &data,
                    version,
                    &mut output[..out_capacity],
                    &mut segments[..segment_capacity]
                ),
                Err(error)
            );
            assert_eq!(
                parse_payload(
                    &data,
                    version,
                    &mut output[..out_capacity],
                    &mut segments[..segment_capacity]
                ),
                Err(error)
            );
        }
    }
    for data in [&[][..], &[0][..], &[0; MAX_DATA_CODEWORDS][..]] {
        assert_eq!(
            parse_payload_info(data, 40, &mut [], &mut []),
            Ok(PayloadInfo {
                payload_len: 0,
                segment_count: 0,
                fnc1: Fnc1::None,
                structured_append: None,
            })
        );
    }
    for (data, version, error) in [
        (&[][..], 0, Error::MalformedPayload),
        (&[][..], 41, Error::MalformedPayload),
        (&[0; MAX_DATA_CODEWORDS + 1][..], 1, Error::MalformedPayload),
        (&[0x40][..], 1, Error::Truncated),
    ] {
        let mut segments = [Segment::EMPTY; 1];
        assert_eq!(
            parse_payload_info(data, version, &mut [], &mut segments),
            Err(error)
        );
        assert_eq!(
            parse_payload(data, version, &mut [], &mut segments),
            Err(error)
        );
    }
}
#[test]
fn eci_lengths_and_application() {
    for (v, prefix, n) in [(26, 26, 8), (300, 0x812c, 16), (70000, 0xc11170, 24)] {
        let (_, segments, _, _) = parse(
            Bits::default()
                .add(7, 4)
                .add(prefix, n)
                .add(4, 4)
                .add(1, 8)
                .add(0xff, 8)
                .add(0, 4),
        )
        .unwrap();
        assert_eq!(segments[0].eci, v);
        assert_eq!(segments[1].eci, v);
    }
}
#[test]
fn malformed_counts_and_control_order() {
    let cases = [
        Bits::default().add(1, 4).add(3, 10).add(1000, 10),
        Bits::default().add(1, 4).add(2, 10).add(100, 7),
        Bits::default().add(1, 4).add(1, 10).add(10, 4),
        Bits::default().add(2, 4).add(2, 9).add(2025, 11),
        Bits::default().add(2, 4).add(1, 9).add(45, 6),
        Bits::default().add(3, 4).add(1, 4).add(0, 4).add(0, 8),
        Bits::default().add(5, 4).add(5, 4),
        Bits::default().add(7, 4).add(0xe0, 8),
        Bits::default().add(7, 4).add(0xcfffff, 24),
        Bits::default().add(8, 4).add(1, 8).add(63, 13),
    ];
    for case in cases {
        assert_eq!(parse(case), Err(Error::MalformedPayload));
    }
    assert_eq!(
        parse(Bits::default().add(6, 4)),
        Err(Error::UnsupportedMode)
    );
    assert_eq!(
        parse(Bits::default().add(4, 4).add(3, 8).add(1, 8)),
        Err(Error::Truncated)
    );
}
#[test]
fn fnc_first_percent_only_in_alphanumeric() {
    let (out, s, f, _) = parse(
        Bits::default()
            .add(5, 4)
            .add(2, 4)
            .add(3, 9)
            .add(38 * 45 + 38, 11)
            .add(38, 6)
            .add(4, 4)
            .add(1, 8)
            .add(37, 8)
            .add(0, 4),
    )
    .unwrap();
    assert_eq!(f, Fnc1::First);
    assert_eq!(out, b"%\x1d%");
    assert_eq!(s[1].len, 2);
}
#[test]
fn fnc_percent_conversion_is_independent_of_alphanumeric_splits() {
    for second in [false, true] {
        for (text, expected) in [
            (b"%".as_slice(), b"\x1d".as_slice()),
            (b"%%", b"%"),
            (b"%%%", b"%\x1d"),
            (b"%%%%", b"%%"),
            (b"A%%%B%%C%D", b"A%\x1dB%C\x1dD"),
        ] {
            // Every possible partition of the same Alphanumeric characters.
            for boundaries in 0..(1 << text.len().saturating_sub(1)) {
                let mut bits = Bits::default().fnc(second);
                let mut start = 0;
                for end in 1..=text.len() {
                    if end == text.len() || boundaries & (1 << (end - 1)) != 0 {
                        bits = bits.alphanumeric(&text[start..end]);
                        start = end;
                    }
                }
                let (out, segments, fnc, _) = parse(bits.add(0, 4)).unwrap();
                assert_eq!(out, expected, "{text:?}, split {boundaries}");
                assert_eq!(
                    fnc,
                    if second {
                        Fnc1::Second(42)
                    } else {
                        Fnc1::First
                    }
                );
                let mut offset = 0;
                for segment in segments {
                    assert_eq!(segment.offset, offset);
                    offset += segment.len;
                }
                assert_eq!(offset, expected.len());
            }
        }
    }
    // Without FNC1 each percent remains a distinct byte.
    assert_eq!(
        parse(Bits::default().alphanumeric(b"%").alphanumeric(b"%"))
            .unwrap()
            .0,
        b"%%"
    );
}
#[test]
fn fnc_pair_ownership_survives_eci_and_empty_alphanumeric() {
    for second in [false, true] {
        let (out, segments, _, _) = parse(
            Bits::default()
                .fnc(second)
                .alphanumeric(b"%")
                .add(7, 4)
                .add(26, 8)
                .alphanumeric(b"")
                .add(7, 4)
                .add(0x812c, 16)
                .alphanumeric(b"%A")
                .alphanumeric(b"%")
                .alphanumeric(b"%")
                .add(0, 4),
        )
        .unwrap();
        assert_eq!(out, b"%A%");
        assert_eq!(
            segments
                .iter()
                .map(|s| (s.mode, s.offset, s.len, s.eci))
                .collect::<Vec<_>>(),
            [
                (
                    if second {
                        Mode::Fnc1Second
                    } else {
                        Mode::Fnc1First
                    },
                    0,
                    0,
                    3
                ),
                (Mode::Alphanumeric, 0, 1, 3),
                (Mode::Eci, 1, 0, 26),
                (Mode::Alphanumeric, 1, 0, 26),
                (Mode::Eci, 1, 0, 300),
                (Mode::Alphanumeric, 1, 1, 300),
                (Mode::Alphanumeric, 2, 1, 300),
                (Mode::Alphanumeric, 3, 0, 300),
            ]
        );
    }
}
#[test]
fn fnc_different_data_modes_end_percent_pairs_even_when_empty() {
    for second in [false, true] {
        for (mode, width, value, data_width, middle) in [
            (1, 10, 7, 4, b"7".as_slice()),
            (4, 8, 37, 8, b"%".as_slice()),
            (8, 8, 1855, 13, b"\x8a\xbf".as_slice()),
        ] {
            for count in [0, 1] {
                let mut bits = Bits::default()
                    .fnc(second)
                    .alphanumeric(b"%")
                    .add(mode, 4)
                    .add(count, width);
                if count == 1 {
                    bits = bits.add(value, data_width);
                }
                let (out, segments, _, _) = parse(bits.alphanumeric(b"%").add(0, 4)).unwrap();
                let mut expected = vec![29];
                if count == 1 {
                    expected.extend_from_slice(middle);
                }
                expected.push(29);
                assert_eq!(out, expected);
                assert_eq!((segments[1].offset, segments[1].len), (0, 1));
                assert_eq!(
                    (segments[2].offset, segments[2].len),
                    (1, expected.len() - 2)
                );
                assert_eq!(
                    (segments[3].offset, segments[3].len),
                    (expected.len() - 1, 1)
                );
            }
        }
    }
}
#[test]
fn fnc_percent_termination_and_capacity_boundaries() {
    for second in [false, true] {
        for bits in [
            Bits::default().fnc(second).alphanumeric(b"%").add(0, 4),
            // One zero bit remains in the final byte (short terminator).
            Bits::default().fnc(second).alphanumeric(b"%"),
            // Exactly 48 / 56 bits, with a final unpaired percent.
            Bits::default()
                .fnc(second)
                .add(1, 4)
                .add(0, 10)
                .alphanumeric(b"%%%"),
        ] {
            let (out, _, _, _) = parse(bits).unwrap();
            assert_eq!(*out.last().unwrap(), 29);
        }
        assert_eq!(
            parse(Bits::default().fnc(second).alphanumeric(b"%").add(1, 1)),
            Err(Error::Truncated)
        );
        let data = Bits::default()
            .fnc(second)
            .alphanumeric(b"%")
            .alphanumeric(b"%")
            .add(0, 4)
            .bytes();
        let mut out = [0; 1];
        let mut segments = [Segment::EMPTY; 3];
        let info = parse_payload_info(&data, 1, &mut out, &mut segments).unwrap();
        assert_eq!((info.payload_len, info.segment_count), (1, 3));
        assert_eq!(out, [b'%']);
        assert_eq!((segments[1].offset, segments[1].len), (0, 1));
        assert_eq!((segments[2].offset, segments[2].len), (1, 0));
        assert_eq!(
            parse_payload_info(&data, 1, &mut [], &mut segments),
            Err(Error::OutputTooSmall)
        );
        assert_eq!(
            parse_payload_info(&data, 1, &mut out, &mut segments[..2]),
            Err(Error::SegmentsTooSmall)
        );
        for bits in [
            Bits::default()
                .fnc(second)
                .alphanumeric(b"%")
                .add(7, 4)
                .add(0x80, 8),
            Bits::default()
                .fnc(second)
                .alphanumeric(b"%")
                .add(2, 4)
                .add(1, 9),
            Bits::default().fnc(second).add(2, 4),
        ] {
            assert_eq!(parse(bits), Err(Error::Truncated));
        }
    }
}
#[test]
fn exact_capacities_and_short_terminator() {
    let data = Bits::default().add(1, 4).add(1, 10).add(1, 4).bytes();
    let mut out = [0];
    let mut s = [Segment::EMPTY];
    assert_eq!(parse_payload(&data, 1, &mut out, &mut s).unwrap().0, 1);
    assert_eq!(out, [b'1']);
    assert_eq!(
        parse_payload(&data, 0, &mut out, &mut s),
        Err(Error::MalformedPayload)
    );
    assert_eq!(
        parse_payload(&data, 1, &mut [], &mut s),
        Err(Error::OutputTooSmall)
    );
    assert_eq!(
        parse_payload(&data, 1, &mut out, &mut []),
        Err(Error::SegmentsTooSmall)
    );
}

#[test]
fn fnc_after_headers_and_before_data() {
    for sa in [false, true] {
        for second in [false, true] {
            let mut b = Bits::default();
            if sa {
                b = b.add(3, 4).add(0, 4).add(1, 4).add(0xa7, 8);
            }
            b = b.add(7, 4).add(26, 8).add(if second { 9 } else { 5 }, 4);
            if second {
                b = b.add(42, 8);
            }
            // A single percent is GS; Byte percent and 0xff remain raw.
            b = b
                .add(2, 4)
                .add(1, 9)
                .add(38, 6)
                .add(4, 4)
                .add(2, 8)
                .add(37, 8)
                .add(255, 8)
                .add(0, 4);
            let (out, segments, fnc, append) = parse(b).unwrap();
            assert_eq!(out, b"\x1d%\xff");
            assert_eq!(
                fnc,
                if second {
                    Fnc1::Second(42)
                } else {
                    Fnc1::First
                }
            );
            assert_eq!(
                append,
                sa.then_some(StructuredAppend {
                    index: 0,
                    total: 2,
                    parity: 0xa7
                })
            );
            let mut modes = vec![];
            if sa {
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
            assert_eq!(segments.iter().map(|s| s.mode).collect::<Vec<_>>(), modes);
            assert!(segments[usize::from(sa)..].iter().all(|s| s.eci == 26));
            assert!(segments[..usize::from(sa)].iter().all(|s| s.eci == 3));
            let data = usize::from(sa) + 2;
            assert_eq!((segments[data].offset, segments[data].len), (0, 1));
            assert_eq!((segments[data + 1].offset, segments[data + 1].len), (1, 2));
            assert!(segments[..data].iter().all(|s| s.offset == 0 && s.len == 0));
        }
    }
}
#[test]
fn fnc_order_boundaries() {
    let (_, segments, f, _) = parse(
        Bits::default()
            .add(5, 4)
            .add(7, 4)
            .add(26, 8)
            .add(4, 4)
            .add(1, 8)
            .add(37, 8)
            .add(0, 4),
    )
    .unwrap();
    assert_eq!(f, Fnc1::First);
    assert_eq!(segments[0].eci, 3);
    assert_eq!(segments[2].eci, 26);
    for first in [5, 9] {
        for next in [5, 9] {
            let mut b = Bits::default().add(first, 4);
            if first == 9 {
                b = b.add(42, 8);
            }
            b = b.add(7, 4).add(26, 8).add(next, 4);
            if next == 9 {
                b = b.add(43, 8);
            }
            assert_eq!(parse(b), Err(Error::MalformedPayload));
        }
    }
    for (mode, width) in [(1, 10), (2, 9), (4, 8), (8, 8)] {
        for count in [0, 1] {
            for fnc in [5, 9] {
                let mut b = Bits::default().add(mode, 4).add(count, width);
                if count == 1 {
                    b = b.add(
                        if mode == 2 { 10 } else { 0 },
                        if mode == 1 {
                            4
                        } else if mode == 2 {
                            6
                        } else if mode == 4 {
                            8
                        } else {
                            13
                        },
                    );
                }
                b = b.add(7, 4).add(26, 8).add(fnc, 4);
                if fnc == 9 {
                    b = b.add(42, 8);
                }
                assert_eq!(parse(b), Err(Error::MalformedPayload));
            }
        }
    }
    for prefix in [
        Bits::default().add(7, 4).add(26, 8),
        Bits::default().add(5, 4),
        Bits::default().add(3, 4).add(0, 4).add(1, 4).add(0xa7, 8),
    ] {
        assert_eq!(
            parse(prefix.add(3, 4).add(0, 4).add(1, 4).add(0xa7, 8)),
            Err(Error::MalformedPayload)
        );
    }
}

#[test]
fn fnc_second_application_indicator_domain() {
    for indicator in 0..=255 {
        let result = parse(
            Bits::default()
                .add(9, 4)
                .add(indicator, 8)
                .add(4, 4)
                .add(2, 8)
                .add(37, 8)
                .add(255, 8)
                .add(0, 4),
        );
        if indicator <= 99 || (165..=190).contains(&indicator) || (197..=222).contains(&indicator) {
            let (out, segments, fnc, _) = result.unwrap();
            assert_eq!(fnc, Fnc1::Second(indicator as u8));
            assert_eq!(out, b"%\xff");
            assert_eq!(segments[0].mode, Mode::Fnc1Second);
            assert_eq!((segments[0].offset, segments[0].len), (0, 0));
            assert_eq!((segments[1].offset, segments[1].len), (0, 2));
        } else {
            assert_eq!(
                result,
                Err(Error::MalformedPayload),
                "indicator {indicator}"
            );
        }
    }
}
#[test]
fn multiple_eci_history_preserves_order_and_raw_bytes() {
    let (out, segments, _, _) = parse(
        Bits::default()
            .add(7, 4)
            .add(26, 8)
            .add(4, 4)
            .add(1, 8)
            .add(255, 8)
            .add(7, 4)
            .add(0x812c, 16)
            .add(7, 4)
            .add(3, 8)
            .add(4, 4)
            .add(2, 8)
            .add(0, 8)
            .add(128, 8)
            .add(0, 4),
    )
    .unwrap();
    assert_eq!(out, [255, 0, 128]);
    assert_eq!(
        segments
            .iter()
            .map(|s| (s.mode, s.eci, s.offset, s.len))
            .collect::<Vec<_>>(),
        [
            (Mode::Eci, 26, 0, 0),
            (Mode::Byte, 26, 0, 1),
            (Mode::Eci, 300, 1, 0),
            (Mode::Eci, 3, 1, 0),
            (Mode::Byte, 3, 1, 2)
        ]
    );
}
