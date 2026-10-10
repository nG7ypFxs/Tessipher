//! Opt-in Micro QR matrix decoding.

use super::{rs, Error, Mode, Segment};

/// Largest Micro QR side length, in modules (M4, 17 modules).
pub const MICRO_MAX_SIDE: usize = 17;
/// Bytes required for a packed M4 matrix.
pub const MICRO_MAX_MATRIX_BYTES: usize = (MICRO_MAX_SIDE * MICRO_MAX_SIDE).div_ceil(8);
/// Largest decoded Micro QR payload in bytes.
pub const MICRO_MAX_PAYLOAD_BYTES: usize = 35;
/// Scratch bytes required by [`decode_micro`].
pub const MICRO_SCRATCH_BYTES: usize = 24 + 255 + rs::WORK_BYTES;

/// Micro QR symbol version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MicroVersion {
    /// 11 by 11 modules; Numeric mode and detection error correction only.
    M1 = 1,
    /// 13 by 13 modules.
    M2 = 2,
    /// 15 by 15 modules.
    M3 = 3,
    /// 17 by 17 modules.
    M4 = 4,
}
impl MicroVersion {
    fn from_side(side: usize) -> Option<Self> {
        match side {
            11 => Some(Self::M1),
            13 => Some(Self::M2),
            15 => Some(Self::M3),
            17 => Some(Self::M4),
            _ => None,
        }
    }
    fn number(self) -> usize {
        self as usize
    }
}

/// Micro QR error-correction level. M1 has detection codewords but no level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MicroEcLevel {
    /// Low redundancy.
    L = 0,
    /// Medium redundancy.
    M = 1,
    /// Quartile redundancy (M4 only).
    Q = 2,
}

/// Validated, borrowed Micro QR module matrix.
#[derive(Clone, Copy)]
pub struct MicroMatrix<'a> {
    bits: &'a [u8],
    side: usize,
    version: MicroVersion,
}
impl<'a> MicroMatrix<'a> {
    /// Borrows packed modules for an M1, M2, M3, or M4 symbol.
    ///
    /// Bits are row-major and most-significant bit first, without row padding.
    /// Exclude the quiet zone. Extra bytes and unused final bits are ignored.
    pub fn new(bits: &'a [u8], side: usize) -> Result<Self, Error> {
        let version = MicroVersion::from_side(side).ok_or(Error::InvalidMatrix)?;
        let needed = side
            .checked_mul(side)
            .ok_or(Error::InvalidMatrix)?
            .div_ceil(8);
        if bits.len() < needed {
            return Err(Error::InvalidMatrix);
        }
        Ok(Self {
            bits,
            side,
            version,
        })
    }
    /// Returns the side length in modules.
    pub fn side(self) -> usize {
        self.side
    }
    /// Returns the Micro QR version implied by the side length.
    pub fn version(self) -> MicroVersion {
        self.version
    }
    /// Returns whether `(x, y)` is dark, or `None` outside the matrix.
    pub fn get(self, x: usize, y: usize) -> Option<bool> {
        (x < self.side && y < self.side).then(|| self.bit(x, y))
    }
    fn bit(self, x: usize, y: usize) -> bool {
        let i = y * self.side + x;
        self.bits[i / 8] & (0x80 >> (i % 8)) != 0
    }
}

/// Metadata for a decoded Micro QR symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MicroDecoded {
    /// Micro QR version M1 through M4.
    pub version: MicroVersion,
    /// Error-correction level; `None` for M1 detection error correction.
    pub ec_level: Option<MicroEcLevel>,
    /// Data mask pattern (0..=3).
    pub mask: u8,
    /// Valid prefix length in the caller's payload buffer.
    pub payload_len: usize,
    /// Valid prefix length in the caller's segment buffer.
    pub segment_count: usize,
    /// Corrected codewords in the symbol's single Reed–Solomon block.
    pub corrected_codewords: u8,
    /// Hamming distance of the selected format information (0..=3).
    pub format_distance: u8,
    /// Clockwise quarter turns used to put the finder at the logical top-left.
    pub rotation: u8,
    /// Whether decoding required reflection after rotation.
    pub mirrored: bool,
}

#[derive(Clone, Copy)]
struct Oriented<'a> {
    matrix: MicroMatrix<'a>,
    transform: u8,
}
impl Oriented<'_> {
    fn bit(self, x: usize, y: usize) -> bool {
        let n = self.matrix.side - 1;
        let (x, y) = match self.transform {
            0 => (x, y),
            1 => (n - y, x),
            2 => (n - x, n - y),
            3 => (y, n - x),
            4 => (y, x),
            5 => (n - x, y),
            6 => (n - y, n - x),
            _ => (x, n - y),
        };
        self.matrix.bit(x, y)
    }
}

/// Decodes an M1–M4 sampled matrix without allocation.
///
/// All four rotations and their reflected forms are tried. Supply at least
/// [`MICRO_SCRATCH_BYTES`] bytes. Only returned payload and segment prefixes
/// are valid; writable buffers are unspecified after failure.
pub fn decode_micro(
    matrix: MicroMatrix<'_>,
    scratch: &mut [u8],
    output: &mut [u8],
    segments: &mut [Segment],
) -> Result<MicroDecoded, Error> {
    if scratch.len() < MICRO_SCRATCH_BYTES {
        return Err(Error::ScratchTooSmall);
    }
    let mut first = None;
    for transform in 0..8 {
        match decode_oriented(Oriented { matrix, transform }, scratch, output, segments) {
            Ok(mut decoded) => {
                decoded.rotation = transform & 3;
                decoded.mirrored = transform >= 4;
                return Ok(decoded);
            }
            Err(e) if capacity_error(e) => return Err(e),
            Err(e) => {
                if first.is_none() {
                    first = Some(e);
                }
            }
        }
    }
    Err(first.unwrap_or(Error::InvalidMatrix))
}

fn capacity_error(error: Error) -> bool {
    matches!(
        error,
        Error::ScratchTooSmall | Error::OutputTooSmall | Error::SegmentsTooSmall
    )
}

fn decode_oriented(
    matrix: Oriented<'_>,
    scratch: &mut [u8],
    output: &mut [u8],
    segments: &mut [Segment],
) -> Result<MicroDecoded, Error> {
    let version = matrix.matrix.version;
    validate_functions(matrix)?;
    let (format, format_distance) = read_format(matrix)?;
    let symbol = format >> 2;
    let expected_version = match symbol {
        0 => MicroVersion::M1,
        1 | 2 => MicroVersion::M2,
        3 | 4 => MicroVersion::M3,
        _ => MicroVersion::M4,
    };
    if version != expected_version {
        return Err(Error::Format);
    }
    let ec_level = match symbol {
        0 => None,
        1 | 3 | 5 => Some(MicroEcLevel::L),
        2 | 4 | 6 => Some(MicroEcLevel::M),
        _ => Some(MicroEcLevel::Q),
    };
    let mask = format & 3;
    let (total, data_bytes, data_bits) = characteristics(symbol);
    let (raw, rest) = scratch.split_at_mut(24);
    let (block, work) = rest.split_at_mut(255);
    raw.fill(0);

    let side = matrix.matrix.side;
    let inc = if matches!(version, MicroVersion::M1 | MicroVersion::M3) {
        2
    } else {
        0
    };
    let total_bits = data_bits + (total - data_bytes) * 8;
    let mut source_bit = 0usize;
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
                if !is_function(side, x, y) {
                    if source_bit < total_bits && (matrix.bit(x, y) ^ micro_mask(mask, x, y)) {
                        let target = if source_bit < data_bits {
                            source_bit
                        } else {
                            data_bytes * 8 + source_bit - data_bits
                        };
                        raw[target / 8] |= 0x80 >> (target % 8);
                    }
                    source_bit += 1;
                }
            }
        }
        right = right.saturating_sub(2);
    }
    if source_bit != total_bits {
        return Err(Error::InvalidMatrix);
    }
    block[..total].copy_from_slice(&raw[..total]);
    // The Micro QR error-correction characteristics reserve p parity symbols
    // against miscorrection. Syndromes still use every ECC codeword.
    let reserve = match symbol {
        0 => 2,         // M1 detection only
        1 => 3,         // M2-L
        2 | 3 | 5 => 2, // M2-M, M3-L, M4-L
        _ => 0,
    };
    let corrected = rs::correct(&mut block[..total], total - data_bytes, reserve, work)? as u8;
    let (payload_len, segment_count) =
        parse_micro(&block[..data_bytes], data_bits, version, output, segments)?;
    Ok(MicroDecoded {
        version,
        ec_level,
        mask,
        payload_len,
        segment_count,
        corrected_codewords: corrected,
        format_distance,
        rotation: 0,
        mirrored: false,
    })
}

fn validate_functions(matrix: Oriented<'_>) -> Result<(), Error> {
    for y in 0..8 {
        for x in 0..8 {
            let expected = if x == 7 || y == 7 {
                false
            } else {
                let edge = x == 0 || y == 0 || x == 6 || y == 6;
                let center = (2..=4).contains(&x) && (2..=4).contains(&y);
                edge || center
            };
            if matrix.bit(x, y) != expected {
                return Err(Error::InvalidMatrix);
            }
        }
    }
    for coordinate in 8..matrix.matrix.side {
        let expected = (coordinate - 8) % 2 == 0;
        if matrix.bit(coordinate, 0) != expected || matrix.bit(0, coordinate) != expected {
            return Err(Error::InvalidMatrix);
        }
    }
    Ok(())
}

fn characteristics(symbol: u8) -> (usize, usize, usize) {
    match symbol {
        0 => (5, 3, 20),
        1 => (10, 5, 40),
        2 => (10, 4, 32),
        3 => (17, 11, 84),
        4 => (17, 9, 68),
        5 => (24, 16, 128),
        6 => (24, 14, 112),
        _ => (24, 10, 80),
    }
}

fn is_function(side: usize, x: usize, y: usize) -> bool {
    x >= side || y >= side || (x < 9 && y < 9) || x == 0 || y == 0
}

fn micro_mask(mask: u8, x: usize, y: usize) -> bool {
    match mask {
        0 => y % 2 == 0,
        1 => (y / 2 + x / 3) % 2 == 0,
        2 => ((x * y) % 2 + (x * y) % 3) % 2 == 0,
        _ => ((x + y) % 2 + (x * y) % 3) % 2 == 0,
    }
}

fn format_word(data: u8) -> u32 {
    let mut rem = data as u32;
    for _ in 0..10 {
        rem = (rem << 1) ^ ((rem >> 9) * 0x537);
    }
    (((data as u32) << 10) | rem) ^ 0x4445
}

fn read_format(matrix: Oriented<'_>) -> Result<(u8, u8), Error> {
    let mut observed = 0u32;
    for i in 0..8 {
        observed |= (matrix.bit(8, i + 1) as u32) << i;
        observed |= (matrix.bit(i + 1, 8) as u32) << (14 - i);
    }
    let mut best = 4;
    let mut value = 0;
    let mut ambiguous = false;
    for candidate in 0..32u8 {
        let distance = (observed ^ format_word(candidate)).count_ones() as u8;
        match distance.cmp(&best) {
            core::cmp::Ordering::Less => {
                best = distance;
                value = candidate;
                ambiguous = false;
            }
            core::cmp::Ordering::Equal => ambiguous = true,
            core::cmp::Ordering::Greater => {}
        }
    }
    if best <= 3 && !ambiguous {
        Ok((value, best))
    } else {
        Err(Error::Format)
    }
}

struct Bits<'a> {
    data: &'a [u8],
    len: usize,
    pos: usize,
}
impl Bits<'_> {
    fn left(&self) -> usize {
        self.len - self.pos
    }
    fn read(&mut self, count: usize) -> Result<u32, Error> {
        if count > self.left() {
            return Err(Error::Truncated);
        }
        let mut value = 0;
        for _ in 0..count {
            value = (value << 1) | ((self.data[self.pos / 8] >> (7 - self.pos % 8)) & 1) as u32;
            self.pos += 1;
        }
        Ok(value)
    }
}

fn emit(output: &mut [u8], len: &mut usize, byte: u8) -> Result<(), Error> {
    if *len >= output.len() {
        return Err(Error::OutputTooSmall);
    }
    output[*len] = byte;
    *len += 1;
    Ok(())
}

fn parse_micro(
    data: &[u8],
    data_bits: usize,
    version: MicroVersion,
    output: &mut [u8],
    segments: &mut [Segment],
) -> Result<(usize, usize), Error> {
    const ALPHA: &[u8; 45] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";
    let v = version.number();
    let mut bits = Bits {
        data,
        len: data_bits,
        pos: 0,
    };
    let mut payload_len = 0;
    let mut segment_count = 0;
    loop {
        let indicator = if version == MicroVersion::M1 {
            if segment_count != 0 {
                break;
            }
            if bits.left() < 3 {
                let left = bits.left();
                if bits.read(left)? != 0 {
                    return Err(Error::Truncated);
                }
                break;
            }
            0
        } else {
            let width = v - 1;
            if bits.left() < width {
                let left = bits.left();
                if bits.read(left)? != 0 {
                    return Err(Error::Truncated);
                }
                break;
            }
            bits.read(width)? as u8
        };
        let (mode, count_width) = match indicator {
            0 => (Mode::Numeric, v + 2),
            1 if v >= 2 => (Mode::Alphanumeric, v + 1),
            2 if v >= 3 => (Mode::Byte, v + 1),
            3 if v >= 3 => (Mode::Kanji, v),
            _ => return Err(Error::UnsupportedMode),
        };
        if bits.left() < count_width {
            let left = bits.left();
            if bits.read(left)? != 0 {
                return Err(Error::Truncated);
            }
            break;
        }
        let mut count = bits.read(count_width)? as usize;
        if count == 0 {
            break;
        }
        if segment_count >= segments.len() {
            return Err(Error::SegmentsTooSmall);
        }
        let start = payload_len;
        match mode {
            Mode::Numeric => {
                while count > 0 {
                    let digits = count.min(3);
                    let width = [0, 4, 7, 10][digits];
                    let value = bits.read(width)?;
                    if value >= [0, 10, 100, 1000][digits] {
                        return Err(Error::MalformedPayload);
                    }
                    if digits == 3 {
                        emit(output, &mut payload_len, b'0' + (value / 100) as u8)?;
                    }
                    if digits >= 2 {
                        emit(output, &mut payload_len, b'0' + (value / 10 % 10) as u8)?;
                    }
                    emit(output, &mut payload_len, b'0' + (value % 10) as u8)?;
                    count -= digits;
                }
            }
            Mode::Alphanumeric => {
                while count >= 2 {
                    let value = bits.read(11)? as usize;
                    if value >= 45 * 45 {
                        return Err(Error::MalformedPayload);
                    }
                    emit(output, &mut payload_len, ALPHA[value / 45])?;
                    emit(output, &mut payload_len, ALPHA[value % 45])?;
                    count -= 2;
                }
                if count == 1 {
                    let value = bits.read(6)? as usize;
                    if value >= 45 {
                        return Err(Error::MalformedPayload);
                    }
                    emit(output, &mut payload_len, ALPHA[value])?;
                }
            }
            Mode::Byte => {
                for _ in 0..count {
                    let value = bits.read(8)? as u8;
                    emit(output, &mut payload_len, value)?;
                }
            }
            Mode::Kanji => {
                for _ in 0..count {
                    let value = bits.read(13)? as u16;
                    let assembled = (value / 0xc0) * 0x100 + value % 0xc0;
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
                    emit(output, &mut payload_len, hi)?;
                    emit(output, &mut payload_len, lo)?;
                }
            }
            _ => return Err(Error::UnsupportedMode),
        }
        segments[segment_count] = Segment {
            mode,
            offset: start,
            len: payload_len - start,
            eci: 3,
        };
        segment_count += 1;
        if version == MicroVersion::M1 {
            break;
        }
    }
    Ok((payload_len, segment_count))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_payload_rejects_truncation_and_malformed_numeric() {
        let mut output = [0; 64];
        let mut segments = [Segment::EMPTY; 4];
        // M4 Byte indicator 010, count 31, with no byte data.
        assert_eq!(
            parse_micro(&[0x5f], 8, MicroVersion::M4, &mut output, &mut segments,),
            Err(Error::Truncated)
        );
        // M1 count 1 followed by the illegal four-bit numeric value 15.
        assert_eq!(
            parse_micro(&[0x3e], 7, MicroVersion::M1, &mut output, &mut segments,),
            Err(Error::MalformedPayload)
        );
        assert_eq!(
            parse_micro(&[0x3e], 6, MicroVersion::M1, &mut output, &mut segments,),
            Err(Error::Truncated)
        );
    }
}
