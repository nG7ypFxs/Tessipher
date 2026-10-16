//! Tracker state transitions and bounded region-of-interest recovery.

use super::region::{decode_region, recognition_failure};
use super::threshold::PixelBudget;
use super::{
    validate_input, Config, Detection, DetectionMode, Error, GrayImage, Point, Rect,
    SamplingStrategy, Segment, Stats,
};

/// A search hint, not proof that a QR symbol exists. It retains no image or payload.
/// Coordinates refer to the complete image passed to [`Tracker::decode_with_stats`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrackHint {
    /// Expanded, clamped search rectangle in input-image coordinates.
    pub roi: Rect,
    /// Width of the input image from which this hint was obtained.
    pub image_width: usize,
    /// Height of the input image from which this hint was obtained.
    pub image_height: usize,
}
/// Frame-to-frame search state. Candidates expire after two complete frame misses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrackerState {
    /// Search the whole input image.
    #[default]
    Searching,
    /// An unconfirmed geometry or previous detection survived one frame miss.
    Candidate {
        /// Search rectangle; it does not confirm QR presence.
        hint: TrackHint,
        /// Consecutive completely searched frames without a decoded symbol.
        misses: u8,
    },
    /// The previous frame successfully decoded at this location.
    Tracking {
        /// Expanded location of the successful symbol.
        hint: TrackHint,
    },
}
/// Frame counters; ROI and full-image searches share one instance and all budgets.
pub type TrackerStats = Stats;

/// Caller-owned, allocation-free tracker containing only geometry and state.
///
/// A saved ROI is searched first, followed by the full image in the same frame
/// after ordinary recognition failure. A full-image ROI is never searched twice.
/// Budget, validation, and capacity failures leave the previous state unchanged.
/// Different image dimensions invalidate the saved hint for that call. For a new
/// crop origin or scale with unchanged dimensions, the caller must call [`Self::reset`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tracker {
    state: TrackerState,
}
impl Tracker {
    /// Creates a tracker in [`TrackerState::Searching`].
    pub const fn new() -> Self {
        Self {
            state: TrackerState::Searching,
        }
    }
    /// Restores a caller-saved state after validating its integer geometry.
    ///
    /// Candidate misses must be exactly one. Saved image dimensions must be
    /// nonzero and at most 32768, with a nonempty rectangle entirely inside
    /// those dimensions. Validation uses saved dimensions, so a later input
    /// resize still follows the ordinary full-image search rule.
    pub fn from_state(state: TrackerState) -> Result<Self, Error> {
        let hint = match state {
            TrackerState::Searching => return Ok(Self::new()),
            TrackerState::Candidate { hint, misses: 1 } | TrackerState::Tracking { hint } => hint,
            TrackerState::Candidate { .. } => return Err(Error::InvalidConfig),
        };
        if hint.image_width == 0
            || hint.image_height == 0
            || hint.image_width > 32768
            || hint.image_height > 32768
            || hint.roi.width == 0
            || hint.roi.height == 0
            || hint
                .roi
                .x
                .checked_add(hint.roi.width)
                .is_none_or(|right| right > hint.image_width)
            || hint
                .roi
                .y
                .checked_add(hint.roi.height)
                .is_none_or(|bottom| bottom > hint.image_height)
        {
            return Err(Error::InvalidImage);
        }
        Ok(Self { state })
    }
    /// Discards previous geometry and starts a full-image search next frame.
    pub fn reset(&mut self) {
        self.state = TrackerState::Searching;
    }
    /// Returns the current state and integer search geometry.
    pub const fn state(&self) -> TrackerState {
        self.state
    }

    /// Decodes one frame in Direct mode with caller-owned buffers and
    /// cumulative frame counters.
    ///
    /// Bounds and buffer requirements are validated against the complete input
    /// before selecting a ROI. Returned corners use input-image coordinates.
    /// A successful decode enters `Tracking`; a first complete miss enters
    /// `Candidate` when a hint exists; a second enters `Searching`. A new failed
    /// candidate cannot reset the miss count. Only a complete recognition failure
    /// or success commits state. Failure buffer contents are unspecified.
    #[allow(clippy::too_many_arguments)]
    pub fn decode_with_stats<const W: usize, const H: usize>(
        &mut self,
        image: GrayImage<'_>,
        config: &Config<W, H>,
        strategy: SamplingStrategy,
        scratch: &mut [u8],
        core_scratch: &mut [u8],
        output: &mut [u8],
        segments: &mut [Segment],
        stats: &mut TrackerStats,
    ) -> Result<Detection, Error> {
        self.decode_sampling_inner(
            image,
            config,
            strategy,
            scratch,
            core_scratch,
            output,
            segments,
            stats,
            None,
            DetectionMode::Direct,
        )
    }
    /// Like [`Self::decode_with_stats`], with an explicit execution mode.
    ///
    /// Each ROI/full-image search applies the selected mode. Fast's dense
    /// fallback, ROI recovery and full-image search all share the frame budgets.
    /// The scratch requirements and state-transition rules are unchanged.
    #[allow(clippy::too_many_arguments)]
    pub fn decode_with_mode<const W: usize, const H: usize>(
        &mut self,
        image: GrayImage<'_>,
        config: &Config<W, H>,
        mode: DetectionMode,
        strategy: SamplingStrategy,
        scratch: &mut [u8],
        core_scratch: &mut [u8],
        output: &mut [u8],
        segments: &mut [Segment],
        stats: &mut TrackerStats,
    ) -> Result<Detection, Error> {
        self.decode_sampling_inner(
            image,
            config,
            strategy,
            scratch,
            core_scratch,
            output,
            segments,
            stats,
            None,
            mode,
        )
    }

    /// Row-cache variant of [`Self::decode_with_stats`]. Uses five full-image-width
    /// rows of internal RAM. ROI and full-frame retries reset tags and share budgets.
    #[allow(clippy::too_many_arguments)]
    pub fn decode_with_stats_cached<const W: usize, const H: usize>(
        &mut self,
        image: GrayImage<'_>,
        config: &Config<W, H>,
        strategy: SamplingStrategy,
        scratch: &mut [u8],
        core_scratch: &mut [u8],
        output: &mut [u8],
        segments: &mut [Segment],
        stats: &mut TrackerStats,
        cache: &mut [u8],
    ) -> Result<Detection, Error> {
        self.decode_sampling_inner(
            image,
            config,
            strategy,
            scratch,
            core_scratch,
            output,
            segments,
            stats,
            Some(cache),
            DetectionMode::Direct,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn decode_sampling_inner<const W: usize, const H: usize>(
        &mut self,
        image: GrayImage<'_>,
        config: &Config<W, H>,
        strategy: SamplingStrategy,
        scratch: &mut [u8],
        core_scratch: &mut [u8],
        output: &mut [u8],
        segments: &mut [Segment],
        stats: &mut TrackerStats,
        mut cache: Option<&mut [u8]>,
        mode: DetectionMode,
    ) -> Result<Detection, Error> {
        *stats = Stats::default();
        validate_input(
            image,
            config,
            scratch,
            core_scratch,
            crate::core::SCRATCH_BYTES,
        )?;
        if let Some(bytes) = cache.as_ref() {
            if bytes.len() < config.local_cache_bytes(image.width)? {
                return Err(Error::ScratchTooSmall);
            }
        }
        let previous = match self.state {
            TrackerState::Candidate { hint, .. } | TrackerState::Tracking { hint }
                if hint.image_width != image.width || hint.image_height != image.height =>
            {
                TrackerState::Searching
            }
            state => state,
        };
        let saved = match previous {
            TrackerState::Searching => None,
            TrackerState::Candidate { hint, .. } | TrackerState::Tracking { hint } => Some(hint),
        };
        let full = Rect {
            x: 0,
            y: 0,
            width: image.width,
            height: image.height,
        };
        let budget = PixelBudget::new(config.max_pixel_reads);
        let mut candidate = None;
        // This closure reborrows every buffer but retains one frame's counters/budget.
        let mut run = |rect: Rect| {
            let region = image.sub_roi(rect)?;
            let mut geometry = None;
            let result = decode_region(
                region,
                config,
                strategy,
                scratch,
                core_scratch,
                output,
                segments,
                stats,
                &budget,
                Some(&mut geometry),
                cache.as_deref_mut(),
                mode,
            );
            let translate = |mut corners: [Point; 4]| {
                for p in &mut corners {
                    p.x += rect.x as f32;
                    p.y += rect.y as f32;
                }
                corners
            };
            if candidate.is_none() {
                if let Some((corners, side)) = geometry {
                    candidate = track_hint(translate(corners), side, image);
                }
            }
            result.map(|mut found| {
                found.corners = translate(found.corners);
                found
            })
        };
        let result = if let Some(hint) = saved.filter(|h| h.roi != full) {
            match run(hint.roi) {
                Err(e) if recognition_failure(e) => run(full),
                result => result,
            }
        } else {
            run(full)
        };
        stats.pixel_reads = budget.used.get();
        match result {
            Ok(mut found) => {
                found.stats = *stats;
                self.state = match track_hint(
                    found.corners,
                    found.decoded.version as usize * 4 + 17,
                    image,
                ) {
                    Some(hint) => TrackerState::Tracking { hint },
                    None => TrackerState::Searching,
                };
                Ok(found)
            }
            Err(e) if recognition_failure(e) => {
                self.state = match previous {
                    TrackerState::Candidate { .. } => TrackerState::Searching,
                    _ => match candidate.or(saved) {
                        Some(hint) => TrackerState::Candidate { hint, misses: 1 },
                        None => TrackerState::Searching,
                    },
                };
                Err(e)
            }
            Err(e) => Err(e),
        }
    }
}
pub(super) fn track_hint(
    corners: [Point; 4],
    side: usize,
    image: GrayImage<'_>,
) -> Option<TrackHint> {
    if corners.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return None;
    }
    let mut min = corners[0];
    let mut max = corners[0];
    let mut edge = 0.0f32;
    let mut area = 0.0f32;
    for i in 0..4 {
        min.x = min.x.min(corners[i].x);
        min.y = min.y.min(corners[i].y);
        max.x = max.x.max(corners[i].x);
        max.y = max.y.max(corners[i].y);
        let next = corners[(i + 1) % 4];
        edge = edge.max(next.sub(corners[i]).norm());
        area += corners[i].cross(next);
    }
    if !edge.is_finite()
        || !area.is_finite()
        || area.abs() < 1.0
        || max.x <= min.x
        || max.y <= min.y
        || side == 0
    {
        return None;
    }
    let margin = (edge * 0.25).max(edge * 4.0 / side as f32).max(8.0);
    let x = (min.x - margin).clamp(0.0, image.width as f32) as usize;
    let y = (min.y - margin).clamp(0.0, image.height as f32) as usize;
    let ceil = |n: f32| {
        let i = n as usize;
        i + usize::from(n > i as f32)
    };
    let right = ceil((max.x + margin).clamp(0.0, image.width as f32));
    let bottom = ceil((max.y + margin).clamp(0.0, image.height as f32));
    if right <= x || bottom <= y {
        return None;
    }
    Some(TrackHint {
        roi: Rect {
            x,
            y,
            width: right - x,
            height: bottom - y,
        },
        image_width: image.width,
        image_height: image.height,
    })
}

#[cfg(test)]
#[path = "tracking_tests.rs"]
mod tests;
