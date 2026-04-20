use qr_core::*;
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
}
type Parsed = (Vec<u8>, Vec<Segment>, Fnc1, Option<StructuredAppend>);
fn parse(b: Bits) -> Result<Parsed, Error> {
    let mut out = vec![0; 7089];
    let mut seg = vec![Segment::EMPTY; 128];
    let (n, s, f, a) = parse_payload(&b.bytes(), 1, &mut out, &mut seg)?;
    out.truncate(n);
    seg.truncate(s);
    Ok((out, seg, f, a))
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
