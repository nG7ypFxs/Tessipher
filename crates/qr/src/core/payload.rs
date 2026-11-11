use super::{Error, Fnc1, Mode, PayloadInfo, Segment, StructuredAppend};
const ALPHA: &[u8; 45] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}
impl Bits<'_> {
    fn left(&self) -> usize {
        self.data.len() * 8 - self.pos
    }
    fn read(&mut self, mut n: usize) -> Result<u32, Error> {
        if n > self.left() {
            return Err(Error::Truncated);
        }
        if n == 8 {
            let index = self.pos / 8;
            let offset = self.pos % 8;
            let mut byte = u32::from(self.data[index]) << offset;
            if offset != 0 {
                byte |= u32::from(self.data[index + 1]) >> (8 - offset);
            }
            self.pos += 8;
            return Ok(byte & 0xff);
        }
        let mut v = 0;
        while n != 0 {
            let available = 8 - self.pos % 8;
            let take = n.min(available);
            let byte = u32::from(self.data[self.pos / 8]);
            v = (v << take) | ((byte >> (available - take)) & ((1 << take) - 1));
            self.pos += take;
            n -= take;
        }
        Ok(v)
    }
}

fn emit(output: &mut [u8], len: &mut usize, b: u8) -> Result<(), Error> {
    if *len >= output.len() {
        return Err(Error::OutputTooSmall);
    }
    output[*len] = b;
    *len += 1;
    Ok(())
}
fn alpha(
    output: &mut [u8],
    len: &mut usize,
    ch: u8,
    fnc: bool,
    pending: &mut bool,
) -> Result<(), Error> {
    if !fnc {
        return emit(output, len, ch);
    }
    if *pending {
        *pending = false;
        if ch == b'%' {
            // The first percent already emitted GS. Keep the pair's byte in
            // that segment, even when a header or segment boundary intervenes.
            output[*len - 1] = b'%';
            return Ok(());
        }
    }
    if ch == b'%' {
        emit(output, len, 29)?;
        *pending = true;
        Ok(())
    } else {
        emit(output, len, ch)
    }
}
pub(crate) fn parse(
    data: &[u8],
    version: u8,
    output: &mut [u8],
    segments: &mut [Segment],
) -> Result<PayloadInfo, Error> {
    let mut bits = Bits { data, pos: 0 };
    let mut r = PayloadInfo {
        payload_len: 0,
        segment_count: 0,
        fnc1: Fnc1::None,
        structured_append: None,
    };
    let mut eci = 3;
    let group = if version <= 9 {
        0
    } else if version <= 26 {
        1
    } else {
        2
    };
    let mut data_started = false;
    let mut pending = false;
    while bits.left() > 0 {
        if bits.left() < 4 {
            let n = bits.left();
            if bits.read(n)? != 0 {
                return Err(Error::Truncated);
            }
            break;
        }
        let indicator = bits.read(4)?;
        if indicator == 0 {
            break;
        }
        let mode = match indicator {
            1 => Mode::Numeric,
            2 => Mode::Alphanumeric,
            3 => Mode::StructuredAppend,
            4 => Mode::Byte,
            5 => Mode::Fnc1First,
            7 => Mode::Eci,
            8 => Mode::Kanji,
            9 => Mode::Fnc1Second,
            _ => return Err(Error::UnsupportedMode),
        };
        if r.segment_count >= segments.len() {
            return Err(Error::SegmentsTooSmall);
        }
        let start = r.payload_len;
        match mode {
            Mode::StructuredAppend => {
                if r.segment_count != 0 {
                    return Err(Error::MalformedPayload);
                }
                let index = bits.read(4)? as u8;
                let total = bits.read(4)? as u8 + 1;
                let parity = bits.read(8)? as u8;
                if index >= total {
                    return Err(Error::MalformedPayload);
                }
                r.structured_append = Some(StructuredAppend {
                    index,
                    total,
                    parity,
                });
            }
            Mode::Fnc1First | Mode::Fnc1Second => {
                if data_started || r.fnc1 != Fnc1::None {
                    return Err(Error::MalformedPayload);
                }
                r.fnc1 = if mode == Mode::Fnc1First {
                    Fnc1::First
                } else {
                    let indicator = bits.read(8)? as u8;
                    if !(indicator <= 99
                        || (165..=190).contains(&indicator)
                        || (197..=222).contains(&indicator))
                    {
                        return Err(Error::MalformedPayload);
                    }
                    Fnc1::Second(indicator)
                };
            }
            Mode::Eci => {
                let first = bits.read(8)?;
                eci = if first & 128 == 0 {
                    first
                } else if first & 192 == 128 {
                    ((first & 63) << 8) | bits.read(8)?
                } else if first & 224 == 192 {
                    ((first & 31) << 16) | bits.read(16)?
                } else {
                    return Err(Error::MalformedPayload);
                };
                if eci > 999_999 {
                    return Err(Error::MalformedPayload);
                }
            }
            _ => {
                data_started = true;
                // Data mode switches end an Alphanumeric escape sequence,
                // including zero-length data segments. ECI emits no data and
                // preserves the sequence, as does an empty Alphanumeric segment.
                if mode != Mode::Alphanumeric {
                    pending = false;
                }
                let widths = match mode {
                    Mode::Numeric => [10, 12, 14],
                    Mode::Alphanumeric => [9, 11, 13],
                    Mode::Byte => [8, 16, 16],
                    Mode::Kanji => [8, 10, 12],
                    _ => unreachable!(),
                };
                let mut count = bits.read(widths[group])? as usize;
                match mode {
                    Mode::Numeric => {
                        while count > 0 {
                            let k = count.min(3);
                            let v = bits.read(match k {
                                3 => 10,
                                2 => 7,
                                _ => 4,
                            })?;
                            if v >= match k {
                                3 => 1000,
                                2 => 100,
                                _ => 10,
                            } {
                                return Err(Error::MalformedPayload);
                            }
                            if k == 3 {
                                emit(output, &mut r.payload_len, b'0' + (v / 100) as u8)?;
                            }
                            if k >= 2 {
                                emit(output, &mut r.payload_len, b'0' + (v / 10 % 10) as u8)?;
                            }
                            emit(output, &mut r.payload_len, b'0' + (v % 10) as u8)?;
                            count -= k;
                        }
                    }
                    Mode::Alphanumeric => {
                        let fnc = r.fnc1 != Fnc1::None;
                        while count >= 2 {
                            let v = bits.read(11)? as usize;
                            if v >= 45 * 45 {
                                return Err(Error::MalformedPayload);
                            }
                            alpha(output, &mut r.payload_len, ALPHA[v / 45], fnc, &mut pending)?;
                            alpha(output, &mut r.payload_len, ALPHA[v % 45], fnc, &mut pending)?;
                            count -= 2;
                        }
                        if count == 1 {
                            let v = bits.read(6)? as usize;
                            if v >= 45 {
                                return Err(Error::MalformedPayload);
                            }
                            alpha(output, &mut r.payload_len, ALPHA[v], fnc, &mut pending)?;
                        }
                    }
                    Mode::Byte => {
                        for _ in 0..count {
                            let b = bits.read(8)? as u8;
                            emit(output, &mut r.payload_len, b)?;
                        }
                    }
                    Mode::Kanji => {
                        for _ in 0..count {
                            let v = bits.read(13)? as u16;
                            let assembled = (v / 0xc0) * 0x100 + v % 0xc0;
                            let sjis = assembled + if assembled < 0x1f00 { 0x8140 } else { 0xc140 };
                            let lo = sjis as u8;
                            let hi = (sjis >> 8) as u8;
                            if !(0x40..=0xfc).contains(&lo)
                                || lo == 0x7f
                                || !((0x81..=0x9f).contains(&hi) || (0xe0..=0xeb).contains(&hi))
                                || sjis > 0xebbf
                            {
                                return Err(Error::MalformedPayload);
                            }
                            emit(output, &mut r.payload_len, hi)?;
                            emit(output, &mut r.payload_len, lo)?;
                        }
                    }
                    _ => unreachable!(),
                }
            }
        }
        segments[r.segment_count] = Segment {
            mode,
            offset: start,
            len: r.payload_len - start,
            eci,
        };
        r.segment_count += 1;
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_chunk_reads_match_bitwise_reference_at_every_offset() {
        let data = [0x00, 0xff, 0xa5, 0x39, 0x81, 0x7e, 0x42];
        for pos in 0..=data.len() * 8 {
            for count in 0..=40 {
                let mut bits = Bits { data: &data, pos };
                if count > data.len() * 8 - pos {
                    assert_eq!(bits.read(count), Err(Error::Truncated));
                    assert_eq!(bits.pos, pos);
                } else {
                    let expected = (pos..pos + count).fold(0u32, |v, bit| {
                        (v << 1) | u32::from((data[bit / 8] >> (7 - bit % 8)) & 1)
                    });
                    assert_eq!(bits.read(count), Ok(expected), "pos={pos}, count={count}");
                    assert_eq!(bits.pos, pos + count);
                }
            }
        }
    }
}
