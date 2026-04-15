use crate::{Error, Fnc1, Mode, Segment, StructuredAppend};
const ALPHA: &[u8; 45] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";
pub(crate) struct Parsed {
    pub len: usize,
    pub segments: usize,
    pub fnc1: Fnc1,
    pub sa: Option<StructuredAppend>,
}
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}
impl Bits<'_> {
    fn left(&self) -> usize {
        self.data.len() * 8 - self.pos
    }
    fn read(&mut self, n: usize) -> Result<u32, Error> {
        if n > self.left() {
            return Err(Error::Truncated);
        }
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | ((self.data[self.pos / 8] >> (7 - self.pos % 8)) & 1) as u32;
            self.pos += 1;
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
            return emit(output, len, b'%');
        }
        emit(output, len, 29)?;
    }
    if ch == b'%' {
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
) -> Result<Parsed, Error> {
    let mut bits = Bits { data, pos: 0 };
    let mut r = Parsed {
        len: 0,
        segments: 0,
        fnc1: Fnc1::None,
        sa: None,
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
        if r.segments >= segments.len() {
            return Err(Error::SegmentsTooSmall);
        }
        let start = r.len;
        match mode {
            Mode::StructuredAppend => {
                if r.segments != 0 {
                    return Err(Error::MalformedPayload);
                }
                let index = bits.read(4)? as u8;
                let total = bits.read(4)? as u8 + 1;
                let parity = bits.read(8)? as u8;
                if index >= total {
                    return Err(Error::MalformedPayload);
                }
                r.sa = Some(StructuredAppend {
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
                                emit(output, &mut r.len, b'0' + (v / 100) as u8)?;
                            }
                            if k >= 2 {
                                emit(output, &mut r.len, b'0' + (v / 10 % 10) as u8)?;
                            }
                            emit(output, &mut r.len, b'0' + (v % 10) as u8)?;
                            count -= k;
                        }
                    }
                    Mode::Alphanumeric => {
                        let fnc = r.fnc1 != Fnc1::None;
                        let mut pending = false;
                        while count >= 2 {
                            let v = bits.read(11)? as usize;
                            if v >= 45 * 45 {
                                return Err(Error::MalformedPayload);
                            }
                            alpha(output, &mut r.len, ALPHA[v / 45], fnc, &mut pending)?;
                            alpha(output, &mut r.len, ALPHA[v % 45], fnc, &mut pending)?;
                            count -= 2;
                        }
                        if count == 1 {
                            let v = bits.read(6)? as usize;
                            if v >= 45 {
                                return Err(Error::MalformedPayload);
                            }
                            alpha(output, &mut r.len, ALPHA[v], fnc, &mut pending)?;
                        }
                        if pending {
                            emit(output, &mut r.len, 29)?;
                        }
                    }
                    Mode::Byte => {
                        for _ in 0..count {
                            let b = bits.read(8)? as u8;
                            emit(output, &mut r.len, b)?;
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
                            emit(output, &mut r.len, hi)?;
                            emit(output, &mut r.len, lo)?;
                        }
                    }
                    _ => unreachable!(),
                }
            }
        }
        segments[r.segments] = Segment {
            mode,
            offset: start,
            len: r.len - start,
            eci,
        };
        r.segments += 1;
    }
    Ok(r)
}
