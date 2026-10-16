//! Finder observations, row scans, and horizontal/vertical cross-checks.

use super::threshold::{Binary, ColumnSummary};
use super::{Error, Point, Stats, FINDER_BYTES};

#[derive(Clone, Copy, Default)]
pub(super) struct Finder {
    pub(super) p: Point,
    pub(super) pitch_x: f32,
    pub(super) pitch_y: f32,
    pub(super) hits: u32,
}
pub(super) fn read_finder(data: &[u8], i: usize) -> Finder {
    let p = i * FINDER_BYTES;
    let f = |n| {
        f32::from_le_bytes([
            data[p + n],
            data[p + n + 1],
            data[p + n + 2],
            data[p + n + 3],
        ])
    };
    Finder {
        p: Point { x: f(0), y: f(4) },
        pitch_x: f(8),
        pitch_y: f(12),
        hits: u32::from_le_bytes([data[p + 16], data[p + 17], data[p + 18], data[p + 19]]),
    }
}
fn write_finder(data: &mut [u8], i: usize, f: Finder) {
    let p = i * FINDER_BYTES;
    data[p..p + 4].copy_from_slice(&f.p.x.to_le_bytes());
    data[p + 4..p + 8].copy_from_slice(&f.p.y.to_le_bytes());
    data[p + 8..p + 12].copy_from_slice(&f.pitch_x.to_le_bytes());
    data[p + 12..p + 16].copy_from_slice(&f.pitch_y.to_le_bytes());
    data[p + 16..p + 20].copy_from_slice(&f.hits.to_le_bytes());
}

fn ratio(r: &[usize; 5]) -> Option<f32> {
    let sum = r.iter().sum::<usize>();
    if sum < 7 || r.contains(&0) {
        return None;
    }
    let m = sum as f32 / 7.0;
    let tolerance = m * 0.85;
    for (i, &n) in r.iter().enumerate() {
        let expected = m * if i == 2 { 3.0 } else { 1.0 };
        if (n as f32 - expected).abs() > tolerance * if i == 2 { 1.6 } else { 1.0 } {
            return None;
        }
    }
    Some(m)
}
// This buffer is a disjoint tail of matrix scratch, live only while scanning.
// Reset for each fixed-x cross-check; tags cannot outlive its image or radius.
struct AxisPacked<'a> {
    bytes: &'a mut [u8],
}
#[derive(Clone, Copy)]
struct AxisSummary {
    sum: u16,
    min: u8,
    max: u8,
}
impl<'a> AxisPacked<'a> {
    fn new(bytes: &'a mut [u8]) -> Self {
        for row in bytes.chunks_exact_mut(6) {
            row[..2].fill(0);
        }
        Self { bytes }
    }
    #[inline]
    fn row<const OPT: bool>(
        &mut self,
        bin: Binary<'_, OPT>,
        x: usize,
        y: usize,
    ) -> Result<AxisSummary, Error> {
        let start = (y % 64) * 6;
        let entry = &self.bytes[start..start + 6];
        let key = (y + 1) as u16;
        if u16::from_le_bytes([entry[0], entry[1]]) == key {
            return Ok(AxisSummary {
                sum: u16::from_le_bytes([entry[2], entry[3]]),
                min: entry[4],
                max: entry[5],
            });
        }
        let mut sum = 0u16;
        let mut min = 255u8;
        let mut max = 0u8;
        for i in -2i32..=2 {
            let px = (x as i32 + i * bin.radius as i32 / 2).clamp(0, bin.image.width as i32 - 1)
                as usize;
            let value = bin.budget.read(bin.image, px, y)?;
            sum += value as u16;
            min = min.min(value);
            max = max.max(value);
        }
        let entry = &mut self.bytes[start..start + 6];
        entry[2..4].copy_from_slice(&sum.to_le_bytes());
        entry[4] = min;
        entry[5] = max;
        entry[..2].copy_from_slice(&key.to_le_bytes());
        Ok(AxisSummary { sum, min, max })
    }
    #[inline]
    fn dark<const OPT: bool>(
        &mut self,
        bin: Binary<'_, OPT>,
        x: usize,
        y: usize,
    ) -> Result<bool, Error> {
        if let Some(dark) = bin.memo_get(x, y) {
            return Ok(dark ^ bin.inverted);
        }
        let pixel = bin.budget.read(bin.image, x, y)? as u32;
        let dark = 'threshold: {
            let mut sum = 0u32;
            let mut min = 255u8;
            let mut max = 0u8;
            for j in -2i32..=2 {
                let py = (y as i32 + j * bin.radius as i32 / 2)
                    .clamp(0, bin.image.height as i32 - 1) as usize;
                let row = self.row(bin, x, py)?;
                sum += row.sum as u32;
                min = min.min(row.min);
                max = max.max(row.max);
                if max - min >= 12 {
                    let remaining = (2 - j) as u32 * 5;
                    let target = pixel * 25;
                    if sum + remaining * bin.range[0] as u32 > target {
                        break 'threshold true;
                    }
                    if sum + remaining * bin.range[1] as u32 <= target {
                        break 'threshold false;
                    }
                }
            }
            if max - min < 12 {
                pixel <= bin.threshold as u32
            } else {
                pixel * 25 < sum
            }
        };
        bin.memo_set(x, y, dark);
        Ok(dark ^ bin.inverted)
    }
}
#[inline(never)]
fn cross_check_vertical_packed<const OPT: bool>(
    bin: Binary<'_, OPT>,
    x: usize,
    y: usize,
    expected: f32,
    bytes: &mut [u8],
) -> Result<Option<(f32, f32)>, Error> {
    let mut rows = AxisPacked::new(bytes);
    if bin.memo.is_some() {
        return cross_check_pixels(bin.image.height, y, expected, |p| rows.dark(bin, x, p));
    }
    // A completed classification at a run boundary can be reused when no
    // image-wide memo is present. Errors never populate this one-point cache.
    let mut last_p = usize::MAX;
    let mut last_dark = false;
    cross_check_pixels(bin.image.height, y, expected, |p| {
        if p == last_p {
            return Ok(last_dark);
        }
        let dark = rows.dark(bin, x, p)?;
        last_p = p;
        last_dark = dark;
        Ok(dark)
    })
}

fn cross_check<const OPT: bool>(
    bin: Binary<'_, OPT>,
    x: usize,
    y: usize,
    horizontal: bool,
    expected: f32,
) -> Result<Option<(f32, f32)>, Error> {
    let limit = if horizontal {
        bin.image.width
    } else {
        bin.image.height
    };
    let center = if horizontal { x } else { y };
    let get = |p: usize| {
        if horizontal {
            bin.dark(p, y)
        } else {
            bin.dark(x, p)
        }
    };
    cross_check_pixels(limit, center, expected, get)
}
// Global scans never need the local threshold or row-cache branches. Select
// their pixel accessor once per cross-check, preserving read order and budgets.
fn cross_check_global<const OPT: bool>(
    bin: Binary<'_, OPT>,
    x: usize,
    y: usize,
    horizontal: bool,
    expected: f32,
) -> Result<Option<(f32, f32)>, Error> {
    let limit = if horizontal {
        bin.image.width
    } else {
        bin.image.height
    };
    let center = if horizontal { x } else { y };
    let get = |p: usize| {
        let pixel = if horizontal {
            bin.budget.read(bin.image, p, y)?
        } else {
            bin.budget.read(bin.image, x, p)?
        };
        Ok((pixel <= bin.threshold) ^ bin.inverted)
    };
    cross_check_pixels(limit, center, expected, get)
}
fn cross_check_pixels(
    limit: usize,
    center: usize,
    expected: f32,
    mut get: impl FnMut(usize) -> Result<bool, Error>,
) -> Result<Option<(f32, f32)>, Error> {
    if !get(center)? {
        return Ok(None);
    }
    let mut r = [0usize; 5];
    let bound = (expected * 6.0 + 5.0) as usize;
    let mut low = center as isize;
    while low >= 0 && get(low as usize)? && r[2] <= bound {
        r[2] += 1;
        low -= 1;
    }
    while low >= 0 && !get(low as usize)? && r[1] <= bound {
        r[1] += 1;
        low -= 1;
    }
    while low >= 0 && get(low as usize)? && r[0] <= bound {
        r[0] += 1;
        low -= 1;
    }
    let mut high = center + 1;
    while high < limit && get(high)? && r[2] <= bound {
        r[2] += 1;
        high += 1;
    }
    let center_high = high;
    while high < limit && !get(high)? && r[3] <= bound {
        r[3] += 1;
        high += 1;
    }
    while high < limit && get(high)? && r[4] <= bound {
        r[4] += 1;
        high += 1;
    }
    let Some(module) = ratio(&r) else {
        return Ok(None);
    };
    if module < expected * 0.4 || module > expected * 2.5 {
        return Ok(None);
    }
    Ok(Some((center_high as f32 - r[2] as f32 * 0.5, module)))
}
fn add_finder(storage: &mut [u8], count: &mut usize, cap: usize, f: Finder) -> bool {
    for i in 0..*count {
        let old = read_finder(storage, i);
        if old.p.sub(f.p).norm() < old.pitch_x.max(old.pitch_y).max(f.pitch_x.max(f.pitch_y)) * 2.0
            && (old.pitch_x - f.pitch_x).abs() < old.pitch_x.max(f.pitch_x)
            && (old.pitch_y - f.pitch_y).abs() < old.pitch_y.max(f.pitch_y)
        {
            let weight = old.hits as f32;
            let divisor = weight + 1.0;
            write_finder(
                storage,
                i,
                Finder {
                    p: old.p.scale(weight).add(f.p).scale(1.0 / divisor),
                    pitch_x: (old.pitch_x * weight + f.pitch_x) / divisor,
                    pitch_y: (old.pitch_y * weight + f.pitch_y) / divisor,
                    hits: old.hits + 1,
                },
            );
            return true;
        }
    }
    if *count >= cap {
        let mut weakest = 0;
        for i in 1..cap {
            if read_finder(storage, i).hits < read_finder(storage, weakest).hits {
                weakest = i;
            }
        }
        if read_finder(storage, weakest).hits <= 1 {
            write_finder(storage, weakest, f);
        }
        return false;
    }
    write_finder(storage, *count, f);
    *count += 1;
    true
}
pub(super) fn scan<const OPT: bool>(
    bin: Binary<'_, OPT>,
    storage: &mut [u8],
    cap: usize,
    step: usize,
    stats: &mut Stats,
    columns: Option<&mut [u8]>,
) -> Result<(usize, bool), Error> {
    if OPT && !bin.local {
        // Keep thresholding and the sequential read counter in the scan loop;
        // only cross-checks need the shared counter and general pixel accessor.
        return scan_global_register(bin, storage, cap, step, stats);
    }
    // Preserve these five rows during irregular cross-check/geometry access.
    // Only the continuous horizontal scan prefetches; other misses read directly.
    let scan_bin = Binary {
        fill_cache: true,
        ..bin
    };
    let (mut columns, mut axis) = if OPT {
        columns.map_or((None, None), |bytes| {
            let (columns, tail) = bytes.split_at_mut(bin.image.width * 4);
            (Some(ColumnSummary::new(columns)), tail.get_mut(..384))
        })
    } else {
        (None, None)
    };
    let mut count = 0;
    let mut overflow = false;
    for y in (0..bin.image.height).step_by(step) {
        stats.rows += 1;
        if let Some(columns) = columns.as_mut() {
            columns.fill(bin, y)?;
        }
        let mut runs = [0usize; 5];
        let mut color = false;
        for x in 0..=bin.image.width {
            let next = x < bin.image.width
                && match columns.as_ref() {
                    Some(columns) => columns.dark(bin, x, y)?,
                    None => scan_bin.dark(x, y)?,
                };
            if next == color {
                runs[4] += 1;
                continue;
            }
            if color {
                if let Some(module) = ratio(&runs) {
                    let cx = x as f32 - runs[4] as f32 - runs[3] as f32 - runs[2] as f32 * 0.5;
                    if cx >= 0.0 && cx < bin.image.width as f32 {
                        let vertical = match axis.as_deref_mut() {
                            Some(bytes) => {
                                cross_check_vertical_packed(bin, cx as usize, y, module, bytes)
                            }
                            None => cross_check(bin, cx as usize, y, false, module),
                        };
                        if let Some((cy, my)) = vertical? {
                            if let Some((rx, mx)) =
                                cross_check(bin, cx as usize, cy as usize, true, my)?
                            {
                                stats.finder_observations += 1;
                                let f = Finder {
                                    p: Point { x: rx, y: cy },
                                    pitch_x: mx,
                                    pitch_y: my,
                                    hits: 1,
                                };
                                if observation_pattern(bin, f)?
                                    && !add_finder(storage, &mut count, cap, f)
                                {
                                    overflow = true;
                                }
                            }
                        }
                    }
                }
            }
            runs.rotate_left(1);
            runs[4] = 1;
            color = next;
        }
    }
    // Stable insertion sort: strongest repeated observation first; ties keep scan order.
    for i in 1..count {
        let value = read_finder(storage, i);
        let mut j = i;
        while j > 0 {
            let prev = read_finder(storage, j - 1);
            if prev.hits >= value.hits {
                break;
            }
            write_finder(storage, j, prev);
            j -= 1;
        }
        write_finder(storage, j, value);
    }
    Ok((count, overflow))
}
// Compare eight byte lanes without borrowing between lanes, then gather MSBs.
#[inline]
#[cfg(target_pointer_width = "64")]
fn global_mask8(pixels: &[u8], threshold: u8) -> u8 {
    let word = u64::from_le_bytes(pixels[..8].try_into().unwrap());
    let high = 0x8080_8080_8080_8080u64;
    let low = 0x7f7f_7f7f_7f7f_7f7fu64;
    let threshold_low = u64::from(threshold & 0x7f) * 0x0101_0101_0101_0101;
    let compare = (threshold_low | high) - (word & low);
    let mask = if threshold < 128 {
        compare & !word
    } else {
        compare | !word
    } & high;
    (mask.wrapping_mul(0x0002_0408_1020_4081) >> 56) as u8
}
fn scan_global_register<const OPT: bool>(
    bin: Binary<'_, OPT>,
    storage: &mut [u8],
    cap: usize,
    step: usize,
    stats: &mut Stats,
) -> Result<(usize, bool), Error> {
    // At most one helper per buffered pixel. Each pair of monotonic axis
    // checks reads at most width + height + 14 pixels; four observation
    // patterns add at most 196. A full batch and all possible helpers must
    // fit before any read-ahead, so budget failures retain scalar positions.
    // Valid detector dimensions are <=32768; this product fits usize on 32-bit targets.
    let batch_guard = 16 * (bin.image.width + bin.image.height + 211);
    let mut count = 0;
    let mut overflow = false;
    for y in (0..bin.image.height).step_by(step) {
        stats.rows += 1;
        let mut used = bin.budget.used.get();
        let mut runs = [0usize; 5];
        let mut color = false;
        let mut x = 0;
        let mut bits = 0u16;
        let mut buffered = 0usize;
        while x <= bin.image.width {
            let next = if x < bin.image.width {
                if buffered == 0
                    && bin.image.width - x >= 16
                    && bin.budget.limit - used >= batch_guard
                {
                    let start = y * bin.image.stride + x;
                    let pixels = &bin.image.data[start..start + 16];
                    #[cfg(target_pointer_width = "64")]
                    {
                        bits = u16::from(global_mask8(pixels, bin.threshold))
                            | (u16::from(global_mask8(&pixels[8..], bin.threshold)) << 8);
                        if bin.inverted {
                            bits = !bits;
                        }
                    }
                    #[cfg(not(target_pointer_width = "64"))]
                    {
                        bits = 0;
                        for (i, &pixel) in pixels.iter().enumerate() {
                            bits |= u16::from((pixel <= bin.threshold) ^ bin.inverted) << i;
                        }
                    }
                    used += 16;
                    buffered = 16;
                }
                if buffered != 0 {
                    let value = bits & 1 != 0;
                    bits >>= 1;
                    buffered -= 1;
                    value
                } else {
                    if used >= bin.budget.limit {
                        bin.budget.used.set(used);
                        return Err(Error::PixelBudget);
                    }
                    let pixel = bin.image.at(x, y);
                    used += 1;
                    (pixel <= bin.threshold) ^ bin.inverted
                }
            } else {
                false
            };
            if next == color {
                let same = if color {
                    (!bits).trailing_zeros()
                } else {
                    bits.trailing_zeros()
                } as usize;
                let skip = same.min(buffered);
                runs[4] += 1 + skip;
                bits >>= skip;
                buffered -= skip;
                x += 1 + skip;
                continue;
            }
            if color {
                if let Some(module) = ratio(&runs) {
                    // Helpers share the counter. Commit sequential scan reads
                    // immediately before entering them, then reload afterward.
                    bin.budget.used.set(used);
                    let cx = x as f32 - runs[4] as f32 - runs[3] as f32 - runs[2] as f32 * 0.5;
                    if cx >= 0.0 && cx < bin.image.width as f32 {
                        if let Some((cy, my)) =
                            cross_check_global(bin, cx as usize, y, false, module)?
                        {
                            if let Some((rx, mx)) =
                                cross_check_global(bin, cx as usize, cy as usize, true, my)?
                            {
                                stats.finder_observations += 1;
                                let f = Finder {
                                    p: Point { x: rx, y: cy },
                                    pitch_x: mx,
                                    pitch_y: my,
                                    hits: 1,
                                };
                                if observation_pattern_global(bin, f)?
                                    && !add_finder(storage, &mut count, cap, f)
                                {
                                    overflow = true;
                                }
                            }
                        }
                    }
                    used = bin.budget.used.get();
                }
            }
            runs.rotate_left(1);
            runs[4] = 1;
            color = next;
            x += 1;
        }
        bin.budget.used.set(used);
    }
    // Stable insertion sort: strongest repeated observation first; ties keep scan order.
    for i in 1..count {
        let value = read_finder(storage, i);
        let mut j = i;
        while j > 0 {
            let prev = read_finder(storage, j - 1);
            if prev.hits >= value.hits {
                break;
            }
            write_finder(storage, j, prev);
            j -= 1;
        }
        write_finder(storage, j, value);
    }
    Ok((count, overflow))
}

fn observation_pattern<const OPT: bool>(bin: Binary<'_, OPT>, f: Finder) -> Result<bool, Error> {
    observation_pattern_pixels(f, |p| bin.point(p))
}
fn observation_pattern_global<const OPT: bool>(
    bin: Binary<'_, OPT>,
    f: Finder,
) -> Result<bool, Error> {
    observation_pattern_pixels(f, |p| bin.global_point(p))
}

fn observation_pattern_pixels(
    f: Finder,
    get: impl Fn(Point) -> Result<Option<bool>, Error>,
) -> Result<bool, Error> {
    for (cos, sin) in [
        (1.0f32, 0.0f32),
        (0.9238795, 0.38268343),
        (0.70710677, 0.70710677),
        (0.38268343, 0.9238795),
    ] {
        let chord_rotation = cos.max(sin);
        let u = Point {
            x: cos * f.pitch_x * chord_rotation,
            y: sin * f.pitch_y * chord_rotation,
        };
        let v = Point {
            x: -sin * f.pitch_x * chord_rotation,
            y: cos * f.pitch_y * chord_rotation,
        };
        let mut good = 0;
        for y in -3i32..=3 {
            for x in -3i32..=3 {
                let edge = x.abs().max(y.abs());
                let expected = edge == 3 || edge <= 1;
                let p = f.p.add(u.scale(x as f32)).add(v.scale(y as f32));
                if get(p)? == Some(expected) {
                    good += 1;
                }
            }
        }
        if good >= 37 {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
#[path = "scan_tests.rs"]
mod tests;
