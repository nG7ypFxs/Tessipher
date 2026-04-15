//! Heap-free QR Model 2 matrix decoder. All variable-size memory belongs to the caller.
//!
//! Matrix storage is packed row-major, most-significant bit first, with no row padding.
//! Scratch is byte-aligned. On failure, scratch and output contents are unspecified.
#![no_std]
#![forbid(unsafe_code)]
mod payload;
mod rs;
mod tables;

pub const MAX_VERSION: u8 = 40;
pub const MAX_SIDE: usize = 177;
pub const MAX_MATRIX_BYTES: usize = (MAX_SIDE * MAX_SIDE).div_ceil(8);
pub const MAX_CODEWORDS: usize = 3706;
pub const MAX_DATA_CODEWORDS: usize = 2956;
pub const MAX_PAYLOAD_BYTES: usize = 7089;
/// Conservative upper bound, including control segments and empty segments.
pub const MAX_SEGMENTS: usize = MAX_DATA_CODEWORDS * 2;
pub const SCRATCH_BYTES: usize = MAX_CODEWORDS + MAX_DATA_CODEWORDS + 255 + rs::WORK_BYTES;
pub const SCRATCH_ALIGN: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Error {
    InvalidMatrix = 1,
    ScratchTooSmall = 2,
    Format = 3,
    Version = 4,
    ReedSolomon = 5,
    Truncated = 6,
    UnsupportedMode = 7,
    MalformedPayload = 8,
    OutputTooSmall = 9,
    SegmentsTooSmall = 10,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum EcLevel {
    L = 0,
    M = 1,
    Q = 2,
    H = 3,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Mode {
    Numeric = 1,
    Alphanumeric = 2,
    StructuredAppend = 3,
    Byte = 4,
    Fnc1First = 5,
    Eci = 7,
    Kanji = 8,
    Fnc1Second = 9,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Segment {
    pub mode: Mode,
    pub offset: usize,
    pub len: usize,
    /// Last ECI assignment encountered, initially 3; not the set of active ECIs.
    /// All ECI controls retain their order and number in the segment list.
    /// No charset, encryption, or compression transformation is performed.
    pub eci: u32,
}
impl Segment {
    pub const EMPTY: Self = Self {
        mode: Mode::Byte,
        offset: 0,
        len: 0,
        eci: 3,
    };
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StructuredAppend {
    pub index: u8,
    pub total: u8,
    pub parity: u8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fnc1 {
    None,
    First,
    Second(u8),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decoded {
    pub version: u8,
    pub ec_level: EcLevel,
    pub mask: u8,
    pub payload_len: usize,
    pub segment_count: usize,
    pub corrected_codewords: u16,
    pub format_distance: u8,
    pub version_distance: u8,
    pub fnc1: Fnc1,
    pub structured_append: Option<StructuredAppend>,
}

/// Validated, borrowed module matrix. A set bit denotes a dark module.
#[derive(Clone, Copy)]
pub struct Matrix<'a> {
    bits: &'a [u8],
    side: usize,
}
impl<'a> Matrix<'a> {
    pub fn new(bits: &'a [u8], side: usize) -> Result<Self, Error> {
        if !(21..=MAX_SIDE).contains(&side)
            || (side - 17) % 4 != 0
            || bits.len() < (side * side).div_ceil(8)
        {
            return Err(Error::InvalidMatrix);
        }
        Ok(Self { bits, side })
    }
    pub fn side(self) -> usize {
        self.side
    }
    pub fn version(self) -> u8 {
        ((self.side - 17) / 4) as u8
    }
    /// Returns `None` outside the matrix.
    pub fn get(self, x: usize, y: usize) -> Option<bool> {
        if x >= self.side || y >= self.side {
            None
        } else {
            Some(self.bit(x, y))
        }
    }
    fn bit(self, x: usize, y: usize) -> bool {
        let i = y * self.side + x;
        self.bits[i / 8] & (0x80 >> (i % 8)) != 0
    }
}

#[derive(Clone, Copy)]
struct OrientedMatrix<'a> {
    matrix: Matrix<'a>,
    transposed: bool,
}
impl OrientedMatrix<'_> {
    fn bit(self, x: usize, y: usize) -> bool {
        if self.transposed {
            self.matrix.bit(y, x)
        } else {
            self.matrix.bit(x, y)
        }
    }
    fn version(self) -> u8 {
        self.matrix.version()
    }
}

fn capacity_error(error: Error) -> bool {
    matches!(
        error,
        Error::ScratchTooSmall | Error::OutputTooSmall | Error::SegmentsTooSmall
    )
}

/// Decodes one sampled matrix, retrying its transpose after a content error.
/// Normal orientation takes priority; capacity errors are returned immediately.
/// Input is borrowed unchanged and both orientations reuse the same scratch.
pub fn decode(
    matrix: Matrix<'_>,
    scratch: &mut [u8],
    output: &mut [u8],
    segments: &mut [Segment],
) -> Result<Decoded, Error> {
    if scratch.len() < SCRATCH_BYTES {
        return Err(Error::ScratchTooSmall);
    }
    let normal = OrientedMatrix {
        matrix,
        transposed: false,
    };
    match decode_oriented(normal, scratch, output, segments) {
        Ok(decoded) => Ok(decoded),
        Err(error) if capacity_error(error) || error == Error::InvalidMatrix => Err(error),
        Err(original_error) => {
            let mirrored = OrientedMatrix {
                matrix,
                transposed: true,
            };
            match decode_oriented(mirrored, scratch, output, segments) {
                Ok(decoded) => Ok(decoded),
                Err(error) if capacity_error(error) => Err(error),
                Err(_) => Err(original_error),
            }
        }
    }
}

fn decode_oriented(
    matrix: OrientedMatrix<'_>,
    scratch: &mut [u8],
    output: &mut [u8],
    segments: &mut [Segment],
) -> Result<Decoded, Error> {
    let version = matrix.version();
    let (format, format_distance) = read_format(matrix)?;
    let version_distance = read_version(matrix)?;
    let ec_level = match format >> 3 {
        1 => EcLevel::L,
        0 => EcLevel::M,
        3 => EcLevel::Q,
        _ => EcLevel::H,
    };
    let mask = format & 7;
    let (raw, rest) = scratch.split_at_mut(MAX_CODEWORDS);
    let (data, rest) = rest.split_at_mut(MAX_DATA_CODEWORDS);
    let (block, work) = rest.split_at_mut(255);
    raw.fill(0);
    let total = raw_codewords(version);
    let mut bit = 0usize;
    let mut right = matrix.matrix.side - 1;
    let mut upwards = true;
    while right > 0 {
        if right == 6 {
            right -= 1;
        }
        for row in 0..matrix.matrix.side {
            let y = if upwards {
                matrix.matrix.side - 1 - row
            } else {
                row
            };
            for x in [right, right - 1] {
                if !is_function(version, x, y) {
                    if bit < total * 8 && (matrix.bit(x, y) ^ mask_bit(mask, x, y)) {
                        raw[bit / 8] |= 0x80 >> (bit % 8);
                    }
                    bit += 1;
                }
            }
        }
        right = right.saturating_sub(2);
        upwards = !upwards;
    }
    if bit / 8 != total {
        return Err(Error::InvalidMatrix);
    }
    let ecc = tables::ECC[ec_level as usize][version as usize] as usize;
    // ISO/IEC 18004:2024 7.5.1 recommends reserving p parity symbols
    // against miscorrection. Syndromes still use all ECC codewords.
    let reserve = match (version, ec_level) {
        (1, EcLevel::L) => 3,
        (1, EcLevel::M) | (2, EcLevel::L) => 2,
        (1, EcLevel::Q | EcLevel::H) | (3, EcLevel::L) => 1,
        _ => 0,
    };
    let blocks = tables::BLOCKS[ec_level as usize][version as usize] as usize;
    let short_len = total / blocks;
    let long_count = total % blocks;
    let short_count = blocks - long_count;
    let short_data = short_len - ecc;
    let data_len = total - blocks * ecc;
    let mut dst = 0;
    let mut corrected = 0;
    for b in 0..blocks {
        let count = short_data + usize::from(b >= short_count);
        for j in 0..short_data {
            block[j] = raw[j * blocks + b];
        }
        if count > short_data {
            block[short_data] = raw[short_data * blocks + b - short_count];
        }
        for j in 0..ecc {
            block[count + j] = raw[data_len + j * blocks + b];
        }
        corrected += rs::correct(&mut block[..count + ecc], ecc, reserve, work)? as u16;
        data[dst..dst + count].copy_from_slice(&block[..count]);
        dst += count;
    }
    let parsed = payload::parse(&data[..data_len], version, output, segments)?;
    Ok(Decoded {
        version,
        ec_level,
        mask,
        payload_len: parsed.len,
        segment_count: parsed.segments,
        corrected_codewords: corrected,
        format_distance,
        version_distance,
        fnc1: parsed.fnc1,
        structured_append: parsed.sa,
    })
}

/// Parses corrected data codewords. Useful for separate demodulators and fuzzing.
/// Version must be 1..=40; terminator and short zero padding are accepted.
pub fn parse_payload(
    data: &[u8],
    version: u8,
    output: &mut [u8],
    segments: &mut [Segment],
) -> Result<(usize, usize, Fnc1, Option<StructuredAppend>), Error> {
    if !(1..=40).contains(&version) || data.len() > MAX_DATA_CODEWORDS {
        return Err(Error::MalformedPayload);
    }
    let r = payload::parse(data, version, output, segments)?;
    Ok((r.len, r.segments, r.fnc1, r.sa))
}

pub fn raw_codewords(version: u8) -> usize {
    if !(1..=40).contains(&version) {
        return 0;
    }
    let v = version as usize;
    let mut n = (16 * v + 128) * v + 64;
    if v >= 2 {
        let align = v / 7 + 2;
        n -= (25 * align - 10) * align - 55;
        if v >= 7 {
            n -= 36;
        }
    }
    n / 8
}
/// Writes alignment center module coordinates; returns their number (0 for V1/invalid version).
pub fn alignment_positions(version: u8, positions: &mut [u8; 7]) -> usize {
    if !(2..=40).contains(&version) {
        return 0;
    }
    let v = version as usize;
    let count = v / 7 + 2;
    let step = if v == 32 {
        26
    } else {
        (v * 4 + count * 2 + 1) / (count * 2 - 2) * 2
    };
    positions[0] = 6;
    for i in 0..count - 1 {
        positions[count - 1 - i] = (v * 4 + 10 - i * step) as u8;
    }
    count
}
/// True for finder/separator/timing/format/version/alignment/dark function modules.
pub fn is_function(version: u8, x: usize, y: usize) -> bool {
    if !(1..=40).contains(&version) {
        return true;
    }
    let side = version as usize * 4 + 17;
    if x >= side || y >= side {
        return true;
    }
    if (x < 9 && y < 9) || (x < 9 && y >= side - 8) || (y < 9 && x >= side - 8) || x == 6 || y == 6
    {
        return true;
    }
    if version >= 7 && ((y < 6 && x >= side - 11) || (x < 6 && y >= side - 11)) {
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
fn mask_bit(mask: u8, x: usize, y: usize) -> bool {
    match mask {
        0 => (x + y) % 2 == 0,
        1 => y % 2 == 0,
        2 => x % 3 == 0,
        3 => (x + y) % 3 == 0,
        4 => (y / 2 + x / 3) % 2 == 0,
        5 => (x * y) % 2 + (x * y) % 3 == 0,
        6 => ((x * y) % 2 + (x * y) % 3) % 2 == 0,
        _ => ((x * y) % 3 + (x + y) % 2) % 2 == 0,
    }
}
fn format_word(data: u8) -> u32 {
    let mut rem = data as u32;
    for _ in 0..10 {
        rem = (rem << 1) ^ ((rem >> 9) * 0x537);
    }
    (((data as u32) << 10) | rem) ^ 0x5412
}
fn version_word(v: u8) -> u32 {
    let mut rem = v as u32;
    for _ in 0..12 {
        rem = (rem << 1) ^ ((rem >> 11) * 0x1f25);
    }
    ((v as u32) << 12) | rem
}
fn read_format(m: OrientedMatrix<'_>) -> Result<(u8, u8), Error> {
    let mut a = 0u32;
    let mut b = 0u32;
    let s = m.matrix.side;
    for i in 0..15 {
        let (x, y) = match i {
            0..=5 => (8, i),
            6 => (8, 7),
            7 => (8, 8),
            8 => (7, 8),
            _ => (14 - i, 8),
        };
        a |= (m.bit(x, y) as u32) << i;
        let (x, y) = if i < 8 {
            (s - 1 - i, 8)
        } else {
            (8, s - 15 + i)
        };
        b |= (m.bit(x, y) as u32) << i;
    }
    nearest(a, b, 0, 31, format_word).ok_or(Error::Format)
}
fn nearest(a: u32, b: u32, lo: u8, hi: u8, word: fn(u8) -> u32) -> Option<(u8, u8)> {
    let mut best = 4;
    let mut value = 0;
    let mut ambiguous = false;
    for v in lo..=hi {
        let code = word(v);
        let d = (a ^ code).count_ones().min((b ^ code).count_ones()) as u8;
        if d < best {
            best = d;
            value = v;
            ambiguous = false;
        } else if d == best {
            ambiguous = true;
        }
    }
    if best <= 3 && !ambiguous {
        Some((value, best))
    } else {
        None
    }
}
fn read_version(m: OrientedMatrix<'_>) -> Result<u8, Error> {
    if m.version() < 7 {
        return Ok(0);
    }
    let mut a = 0;
    let mut b = 0;
    for i in 0..18 {
        let x = m.matrix.side - 11 + i % 3;
        let y = i / 3;
        a |= (m.bit(x, y) as u32) << i;
        b |= (m.bit(y, x) as u32) << i;
    }
    match nearest(a, b, 7, 40, version_word) {
        Some((v, d)) if v == m.version() => Ok(d),
        _ => Err(Error::Version),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrored_valid_format_still_retries_after_content_failure() {
        let corpus = include_bytes!("../../../fixtures/all-model2.bin");
        let mut cursor = 4;
        let mut beyond_format = 0;
        let mut scratch = [0; SCRATCH_BYTES];
        let mut output = [0; 4];
        let mut segments = [Segment::EMPTY; 1];
        // V1 covers every ECC/mask combination and has no version BCH.
        for _ in 0..32 {
            let len = u16::from_le_bytes([corpus[cursor + 3], corpus[cursor + 4]]) as usize;
            cursor += 5;
            let original = Matrix::new(&corpus[cursor..cursor + len], 21).unwrap();
            let mut bits = [0; 56];
            for y in 0..21 {
                for x in 0..21 {
                    if original.bit(y, x) {
                        let i = y * 21 + x;
                        bits[i / 8] |= 128 >> (i % 8);
                    }
                }
            }
            let matrix = Matrix::new(&bits, 21).unwrap();
            let view = OrientedMatrix {
                matrix,
                transposed: false,
            };
            if read_format(view).is_ok() {
                assert_eq!(
                    decode_oriented(view, &mut scratch, &mut output, &mut segments),
                    Err(Error::ReedSolomon)
                );
                decode(matrix, &mut scratch, &mut output, &mut segments).unwrap();
                beyond_format += 1;
            }
            cursor += len;
        }
        assert!(beyond_format > 0);
    }

    #[test]
    fn both_content_failures_preserve_normal_error() {
        let corpus = include_bytes!("../../../fixtures/all-model2.bin");
        let mut cursor = 4;
        let mut bits = [0; 56];
        loop {
            let len = u16::from_le_bytes([corpus[cursor + 3], corpus[cursor + 4]]) as usize;
            assert_eq!(corpus[cursor], 1);
            cursor += 5;
            bits.copy_from_slice(&corpus[cursor..cursor + len]);
            let matrix = Matrix::new(&bits, 21).unwrap();
            if read_format(OrientedMatrix {
                matrix,
                transposed: true,
            })
            .is_err()
            {
                break;
            }
            cursor += len;
        }
        // Keep function patterns, destroy data. The transpose must not hide
        // the original RS diagnostic behind its different format failure.
        for y in 0..21 {
            for x in 0..21 {
                if !is_function(1, x, y) {
                    let i = y * 21 + x;
                    bits[i / 8] &= !(128 >> (i % 8));
                }
            }
        }
        let matrix = Matrix::new(&bits, 21).unwrap();
        let mut scratch = [0; SCRATCH_BYTES];
        let mut output = [0; 32];
        let mut segments = [Segment::EMPTY; 8];
        let normal = decode_oriented(
            OrientedMatrix {
                matrix,
                transposed: false,
            },
            &mut scratch,
            &mut output,
            &mut segments,
        );
        let mirrored = decode_oriented(
            OrientedMatrix {
                matrix,
                transposed: true,
            },
            &mut scratch,
            &mut output,
            &mut segments,
        );
        assert_eq!(normal, Err(Error::ReedSolomon));
        assert_eq!(mirrored, Err(Error::Format));
        assert_eq!(
            decode(matrix, &mut scratch, &mut output, &mut segments),
            normal
        );
    }

    #[test]
    fn bch_uncorrectable_matrix_copy_recovers_from_other_copy() {
        let original = include_bytes!("../../../fixtures/v7.bin");
        let bad_format = (0..32768)
            .find(|&value| nearest(value, value, 0, 31, format_word).is_none())
            .unwrap();
        let bad_version = 0;
        assert_eq!(nearest(bad_version, bad_version, 7, 40, version_word), None);
        for copy in 0..2 {
            let mut bits = *original;
            let mut set = |x: usize, y: usize, value: bool| {
                let i = y * 45 + x;
                bits[i / 8] = (bits[i / 8] & !(128 >> (i % 8))) | ((value as u8) << (7 - i % 8));
            };
            for i in 0..15 {
                let (x, y) = if copy == 0 {
                    match i {
                        0..=5 => (8, i),
                        6 => (8, 7),
                        7 => (8, 8),
                        8 => (7, 8),
                        _ => (14 - i, 8),
                    }
                } else if i < 8 {
                    (44 - i, 8)
                } else {
                    (8, 30 + i)
                };
                set(x, y, bad_format & (1 << i) != 0);
            }
            for i in 0..18 {
                let (x, y) = (34 + i % 3, i / 3);
                let (x, y) = if copy == 0 { (x, y) } else { (y, x) };
                set(x, y, false);
            }
            let mut scratch = [0; SCRATCH_BYTES];
            let mut output = [0; 4];
            let mut segments = [Segment::EMPTY; 1];
            let result = decode(
                Matrix::new(&bits, 45).unwrap(),
                &mut scratch,
                &mut output,
                &mut segments,
            )
            .unwrap();
            assert_eq!(output, *b"TEST");
            assert_eq!((result.format_distance, result.version_distance), (0, 0));
        }
    }

    #[test]
    fn bch_recovers_either_copy_and_rejects_conflicts() {
        for (lo, hi, word) in [(0, 31, format_word as fn(u8) -> u32), (7, 40, version_word)] {
            let bad = (0..32768)
                .find(|&value| nearest(value, value, lo, hi, word).is_none())
                .unwrap();
            for valid in lo..=hi {
                assert_eq!(nearest(bad, word(valid), lo, hi, word), Some((valid, 0)));
                assert_eq!(nearest(word(valid), bad, lo, hi, word), Some((valid, 0)));
            }
            assert_eq!(nearest(word(lo), word(hi), lo, hi, word), None);
        }
    }
}
