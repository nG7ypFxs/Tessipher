//! Threshold-pass scheduling, shared search budgets, and Fast fallback.

use super::sampling::SearchContext;
use super::scan::{read_finder, scan};
use super::threshold::{Binary, PixelBudget, RowCache};
use super::{
    Config, Detection, DetectionMode, Error, GrayImage, Point, SamplingStrategy, Segment, Stats,
    FINDER_BYTES,
};
use crate::core::MAX_MATRIX_BYTES;
use core::cell::{Cell, RefCell};

#[inline(never)]
fn complete_range(image: GrayImage<'_>) -> ([u8; 2], usize) {
    let mut min = 255u8;
    let mut max = 0u8;
    let mut rows = 0;
    for y in 0..image.height {
        let start = y * image.stride;
        for &value in &image.data[start..start + image.width] {
            min = min.min(value);
            max = max.max(value);
        }
        rows += 1;
        // No unread Gray8 value can extend the complete intensity range.
        // Check between rows so each contiguous reduction stays vectorizable.
        if min == 0 && max == 255 {
            break;
        }
    }
    ([min, max], rows * image.width)
}

// Scoped to one immutable ROI and one decode_region call only.
#[derive(Default)]
struct RegionState {
    range: Option<[u8; 2]>,
    memo_ready: bool,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn decode_region<const W: usize, const H: usize>(
    image: GrayImage<'_>,
    config: &Config<W, H>,
    strategy: SamplingStrategy,
    scratch: &mut [u8],
    core_scratch: &mut [u8],
    output: &mut [u8],
    segments: &mut [Segment],
    stats: &mut Stats,
    budget: &PixelBudget,
    mut candidate_hint: Option<&mut Option<([Point; 4], usize)>>,
    cache: Option<&mut [u8]>,
    mode: DetectionMode,
) -> Result<Detection, Error> {
    let mut state = RegionState::default();
    // Dedicated row-cache APIs preserve their input-read order and cache-fill
    // semantics; public execution modes use the optimized uncached path below.
    if cache.is_some() {
        return decode_inner::<W, H, false>(
            image,
            config,
            strategy,
            scratch,
            core_scratch,
            output,
            segments,
            stats,
            budget,
            candidate_hint,
            cache,
            false,
            &mut state,
        );
    }
    let coarse = Config {
        row_step: if mode == DetectionMode::Fast {
            config.row_step.max(3)
        } else {
            config.row_step
        },
        ..*config
    };
    let passes_before = stats.passes;
    let result = decode_inner::<W, H, true>(
        image,
        &coarse,
        strategy,
        scratch,
        core_scratch,
        output,
        segments,
        stats,
        budget,
        candidate_hint.as_deref_mut(),
        None,
        mode == DetectionMode::Fast,
        &mut state,
    );
    match result {
        Err(error)
            if coarse.row_step != config.row_step
                && stats.passes > passes_before
                && recognition_failure(error) =>
        {
            decode_inner::<W, H, true>(
                image,
                config,
                strategy,
                scratch,
                core_scratch,
                output,
                segments,
                stats,
                budget,
                candidate_hint,
                None,
                mode == DetectionMode::Fast,
                &mut state,
            )
        }
        result => result,
    }
}

#[allow(clippy::too_many_arguments)]
fn decode_inner<const W: usize, const H: usize, const OPT: bool>(
    image: GrayImage<'_>,
    config: &Config<W, H>,
    strategy: SamplingStrategy,
    scratch: &mut [u8],
    core_scratch: &mut [u8],
    output: &mut [u8],
    segments: &mut [Segment],
    stats: &mut Stats,
    budget: &PixelBudget,
    candidate_hint: Option<&mut Option<([Point; 4], usize)>>,
    cache: Option<&mut [u8]>,
    fast: bool,
    state: &mut RegionState,
) -> Result<Detection, Error> {
    let cache = cache.map(|bytes| RefCell::new(RowCache::new(bytes)));
    let (matrix, remaining) = scratch.split_at_mut(MAX_MATRIX_BYTES);
    let (finders, tail) = remaining.split_at_mut(config.max_finders * FINDER_BYTES);
    let memo_bytes = (image.width * image.height).div_ceil(4);
    let memo = if fast && tail.len() >= memo_bytes {
        Some(Cell::from_mut(&mut tail[..memo_bytes]).as_slice_of_cells())
    } else {
        None
    };
    let [min, max] = if let Some(range) = state.range {
        range
    } else {
        let mut min = 255u8;
        let mut max = 0u8;
        let used = budget.used.get();
        let reads = image.width * image.height;
        if OPT && reads <= budget.limit - used {
            // No callbacks or fallible operations inside a complete ROI range scan.
            // Keep the partial-budget path scalar to preserve failure positions.
            let (range, actual_reads) = complete_range(image);
            [min, max] = range;
            budget.used.set(used + actual_reads);
        } else {
            for y in 0..image.height {
                for x in 0..image.width {
                    let value = budget.read(image, x, y)?;
                    min = min.min(value);
                    max = max.max(value);
                }
            }
        }
        let range = [min, max];
        state.range = Some(range);
        range
    };
    if max - min < 8 {
        return Err(Error::NotFound);
    }
    let threshold = ((min as u16 + max as u16) / 2) as u8;
    let mut overflow = false;
    let mut search = SearchContext::<OPT> {
        matrix,
        core_scratch,
        output,
        segments,
        stats,
        max_candidates: config.max_candidates,
        max_retries: config.max_retries,
        last_error: None,
        strategy,
        candidate_hint,
    };
    // Fast tries nearby global thresholds before the sampled local mean.
    // Keep the original midrange for local low-contrast fallback and memo reuse.
    let refine = OPT && fast && max - min >= 16;
    let local_start = if refine { 6 } else { 2 };
    for pass in 0..local_start + 2 {
        let threshold = match pass {
            2 | 3 if refine => threshold - (max - min) / 16,
            4 | 5 if refine => threshold + (max - min) / 16,
            _ => threshold,
        };
        if pass == local_start && !state.memo_ready {
            if let Some(memo) = memo {
                for byte in memo {
                    byte.set(0);
                }
                state.memo_ready = true;
            }
        }
        search.stats.passes += 1;
        let bin = Binary::<OPT> {
            image,
            budget,
            cache: cache.as_ref(),
            memo,
            fill_cache: false,
            threshold,
            inverted: pass % 2 != 0,
            local: pass >= local_start,
            range: [min, max],
            radius: config.local_radius,
        };
        let (count, full) = scan(
            bin,
            finders,
            config.max_finders,
            config.row_step,
            search.stats,
            if OPT && bin.local && image.width <= MAX_MATRIX_BYTES / 4 {
                Some(
                    &mut search.matrix[..if fast {
                        image.width * 4
                    } else {
                        MAX_MATRIX_BYTES
                    }],
                )
            } else {
                None
            },
        )?;
        overflow |= full;
        // Keep failed nearby-threshold probes from spending many sampling
        // attempts on projective sweeps. All attempts still share the config limit.
        let probe = refine && (2..6).contains(&pass);
        search.max_retries = if probe {
            config
                .max_retries
                .min(search.stats.retries.saturating_add(2))
        } else {
            config.max_retries
        };
        'candidates: for a in 0..count {
            for b in a + 1..count {
                for c in b + 1..count {
                    let set = [
                        read_finder(finders, a),
                        read_finder(finders, b),
                        read_finder(finders, c),
                    ];
                    match search.candidate(bin, set) {
                        Ok(Some(found)) => return Ok(found),
                        Ok(None) => {}
                        Err(Error::RetryBudget)
                            if probe && search.stats.retries < config.max_retries =>
                        {
                            break 'candidates
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
        }
    }
    if overflow {
        Err(Error::FinderCapacity)
    } else if let Some(e) = search.last_error {
        Err(Error::Decode(e))
    } else {
        Err(Error::NotFound)
    }
}

pub(super) fn recognition_failure(e: Error) -> bool {
    matches!(
        e,
        Error::NotFound
            | Error::Decode(
                crate::core::Error::Format
                    | crate::core::Error::Version
                    | crate::core::Error::ReedSolomon
                    | crate::core::Error::Truncated
                    | crate::core::Error::MalformedPayload
                    | crate::core::Error::UnsupportedMode
            )
    )
}

#[cfg(test)]
#[path = "region_tests.rs"]
mod tests;
