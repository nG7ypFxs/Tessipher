//! Pixel-read budgets, local thresholding, and caller-owned caches.

use super::{Error, GrayImage, Point};
use core::cell::{Cell, RefCell};

/// One counter shared by min/max and every detector pass/helper.
pub(super) struct PixelBudget {
    pub(super) limit: usize,
    pub(super) used: Cell<usize>,
}
impl PixelBudget {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            limit,
            used: Cell::new(0),
        }
    }
    pub(super) fn read(&self, image: GrayImage<'_>, x: usize, y: usize) -> Result<u8, Error> {
        let used = self.used.get();
        if used >= self.limit {
            return Err(Error::PixelBudget);
        }
        // Check before reading; used + 1 is safe even when limit == usize::MAX.
        self.used.set(used + 1);
        Ok(image.at(x, y))
    }
}
// Tags are scoped to one decode region. Incomplete fills never become valid.
pub(super) struct RowCache<'a> {
    bytes: &'a mut [u8],
    tags: [Option<usize>; 5],
    order: [usize; 5],
}
impl<'a> RowCache<'a> {
    pub(super) fn new(bytes: &'a mut [u8]) -> Self {
        Self {
            bytes,
            tags: [None; 5],
            order: [0, 1, 2, 3, 4],
        }
    }
    fn read(
        &mut self,
        image: GrayImage<'_>,
        budget: &PixelBudget,
        x: usize,
        y: usize,
        fill: bool,
    ) -> Result<u8, Error> {
        let slot = match self.tags.iter().position(|tag| *tag == Some(y)) {
            Some(slot) => slot,
            None if !fill => return budget.read(image, x, y),
            None => {
                let slot = self.order[0];
                self.tags[slot] = None;
                let start = slot * image.width;
                for col in 0..image.width {
                    self.bytes[start + col] = budget.read(image, col, y)?;
                }
                self.tags[slot] = Some(y);
                slot
            }
        };
        if fill {
            let position = self.order.iter().position(|&n| n == slot).unwrap();
            self.order[position..].rotate_left(1);
        }
        Ok(self.bytes[slot * image.width + x])
    }
}
// Byte-packed summaries reuse matrix scratch only while scanning. The matrix
// is not sampled until the scan has finished, so their lifetimes never overlap.
// No alignment beyond the detector's byte-aligned scratch is required.
pub(super) struct ColumnSummary<'a> {
    bytes: &'a mut [u8],
}
impl<'a> ColumnSummary<'a> {
    pub(super) fn new(bytes: &'a mut [u8]) -> Self {
        Self { bytes }
    }
    pub(super) fn fill<const OPT: bool>(
        &mut self,
        bin: Binary<'_, OPT>,
        y: usize,
    ) -> Result<(), Error> {
        let used = bin.budget.used.get();
        let reads = bin.image.width * 5;
        // A complete fill has no helper callbacks or fallible reads. Charge
        // it once and zip validated rows so the compiler can process columns
        // in parallel. Keep the scalar path for exact partial-budget behavior.
        if OPT && reads <= bin.budget.limit - used {
            let rows: [&[u8]; 5] = core::array::from_fn(|i| {
                let py = (y as i32 + (i as i32 - 2) * bin.radius as i32 / 2)
                    .clamp(0, bin.image.height as i32 - 1) as usize;
                let start = py * bin.image.stride;
                &bin.image.data[start..start + bin.image.width]
            });
            bin.budget.used.set(used + reads);
            for (out, ((((&a, &b), &c), &d), &e)) in self.bytes.chunks_exact_mut(4).zip(
                rows[0]
                    .iter()
                    .zip(rows[1])
                    .zip(rows[2])
                    .zip(rows[3])
                    .zip(rows[4]),
            ) {
                let sum = a as u16 + b as u16 + c as u16 + d as u16 + e as u16;
                out[..2].copy_from_slice(&sum.to_le_bytes());
                out[2] = a.min(b).min(c).min(d).min(e);
                out[3] = a.max(b).max(c).max(d).max(e);
            }
            return Ok(());
        }
        for x in 0..bin.image.width {
            let mut sum = 0u16;
            let mut min = 255;
            let mut max = 0;
            for j in -2i32..=2 {
                // Divide after multiplication: odd radii use truncation toward
                // zero, exactly as in the direct sampled threshold.
                let py = (y as i32 + j * bin.radius as i32 / 2)
                    .clamp(0, bin.image.height as i32 - 1) as usize;
                let value = bin.budget.read(bin.image, x, py)?;
                sum += value as u16;
                min = min.min(value);
                max = max.max(value);
            }
            let start = x * 4;
            self.bytes[start..start + 2].copy_from_slice(&sum.to_le_bytes());
            self.bytes[start + 2] = min;
            self.bytes[start + 3] = max;
        }
        Ok(())
    }
    #[inline]
    pub(super) fn dark<const OPT: bool>(
        &self,
        bin: Binary<'_, OPT>,
        x: usize,
        y: usize,
    ) -> Result<bool, Error> {
        if let Some(dark) = bin.memo_get(x, y) {
            return Ok(dark ^ bin.inverted);
        }
        let pixel = bin.budget.read(bin.image, x, y)? as u32;
        let records: [u32; 5] = core::array::from_fn(|i| {
            let px = (x as i32 + (i as i32 - 2) * bin.radius as i32 / 2)
                .clamp(0, bin.image.width as i32 - 1) as usize;
            let start = px * 4;
            let bytes = &self.bytes[start..start + 4];
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        });
        let sum: u32 = records.iter().map(|record| record & 0xffff).sum();
        let mean_dark = pixel * 25 < sum;
        let fallback_dark = pixel <= bin.threshold as u32;
        // Only contrast selects between these two decisions; when they agree,
        // its value cannot affect classification.
        let dark = if mean_dark == fallback_dark {
            mean_dark
        } else {
            let mut min = 255u8;
            let mut max = 0u8;
            for record in records {
                min = min.min((record >> 16) as u8);
                max = max.max((record >> 24) as u8);
            }
            if max - min < 12 {
                fallback_dark
            } else {
                mean_dark
            }
        };
        bin.memo_set(x, y, dark);
        Ok(dark ^ bin.inverted)
    }
}
#[derive(Clone, Copy)]
pub(super) struct Binary<'a, const OPT: bool = false> {
    pub(super) image: GrayImage<'a>,
    pub(super) budget: &'a PixelBudget,
    pub(super) cache: Option<&'a RefCell<RowCache<'a>>>,
    // Two bits per ROI pixel: 0 unknown, 1 dark, 2 light. Store un-inverted
    // decisions only after a complete successful threshold calculation.
    pub(super) memo: Option<&'a [Cell<u8>]>,
    pub(super) fill_cache: bool,
    pub(super) threshold: u8,
    pub(super) inverted: bool,
    pub(super) local: bool,
    // Conservative bounds on every pixel in this ROI, from its min/max pass.
    pub(super) range: [u8; 2],
    pub(super) radius: usize,
}
impl<const OPT: bool> Binary<'_, OPT> {
    #[inline]
    pub(super) fn memo_get(self, x: usize, y: usize) -> Option<bool> {
        let bytes = self.memo?;
        let index = y * self.image.width + x;
        let value = (bytes[index / 4].get() >> ((index % 4) * 2)) & 3;
        (value != 0).then_some(value == 1)
    }
    #[inline]
    pub(super) fn memo_set(self, x: usize, y: usize, dark: bool) {
        if let Some(bytes) = self.memo {
            let index = y * self.image.width + x;
            let byte = &bytes[index / 4];
            let shift = (index % 4) * 2;
            byte.set((byte.get() & !(3 << shift)) | ((if dark { 1 } else { 2 }) << shift));
        }
    }

    #[inline]
    fn read(self, x: usize, y: usize) -> Result<u8, Error> {
        if self.local {
            if let Some(cache) = self.cache {
                return cache
                    .borrow_mut()
                    .read(self.image, self.budget, x, y, self.fill_cache);
            }
        }
        self.budget.read(self.image, x, y)
    }
    #[inline]
    pub(super) fn dark(self, x: usize, y: usize) -> Result<bool, Error> {
        if OPT && self.local {
            if let Some(dark) = self.memo_get(x, y) {
                return Ok(dark ^ self.inverted);
            }
        }
        let pixel = self.read(x, y)? as u32;
        let dark = if !self.local {
            pixel <= self.threshold as u32
        } else {
            'threshold: {
                // A fixed 5x5 sample window with no integral-image buffer.
                let mut sum = 0u32;
                let mut min = 255u8;
                let mut max = 0u8;
                for j in -2i32..=2 {
                    for i in -2i32..=2 {
                        let px = (x as i32 + i * self.radius as i32 / 2)
                            .clamp(0, self.image.width as i32 - 1)
                            as usize;
                        let py = (y as i32 + j * self.radius as i32 / 2)
                            .clamp(0, self.image.height as i32 - 1)
                            as usize;
                        let v = self.read(px, py)?;
                        sum += v as u32;
                        min = min.min(v);
                        max = max.max(v);
                    }
                    // Once contrast reaches 12 it cannot fall again. The sum can
                    // grow within the image extrema for each unread sample, so these
                    // bounds prove the final strict mean comparison exactly.
                    if OPT && max - min >= 12 {
                        let target = pixel * 25;
                        let remaining = (2 - j) as u32 * 5;
                        if sum + remaining * self.range[0] as u32 > target {
                            break 'threshold true;
                        }
                        if sum + remaining * self.range[1] as u32 <= target {
                            break 'threshold false;
                        }
                    }
                }
                if max - min < 12 {
                    pixel <= self.threshold as u32
                } else {
                    pixel * 25 < sum
                }
            }
        };
        if OPT && self.local {
            self.memo_set(x, y, dark);
        }
        Ok(dark ^ self.inverted)
    }

    #[inline]
    pub(super) fn global_point(self, p: Point) -> Result<Option<bool>, Error> {
        if !p.x.is_finite()
            || !p.y.is_finite()
            || p.x < 0.0
            || p.y < 0.0
            || p.x >= self.image.width as f32
            || p.y >= self.image.height as f32
        {
            Ok(None)
        } else {
            self.budget
                .read(self.image, p.x as usize, p.y as usize)
                .map(|value| Some((value <= self.threshold) ^ self.inverted))
        }
    }
    #[inline]
    pub(super) fn point(self, p: Point) -> Result<Option<bool>, Error> {
        if !p.x.is_finite()
            || !p.y.is_finite()
            || p.x < 0.0
            || p.y < 0.0
            || p.x >= self.image.width as f32
            || p.y >= self.image.height as f32
        {
            Ok(None)
        } else {
            Ok(Some(self.dark(p.x as usize, p.y as usize)?))
        }
    }
}

#[cfg(test)]
#[path = "threshold_tests.rs"]
mod tests;
