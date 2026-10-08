# Gray8 detection search envelope

## Priority and budgets

Threshold and polarity passes run in this order: global midrange normal, global midrange inverted, local mean normal, and local mean inverted.
The global midrange is the midpoint of the image's minimum and maximum pixel values.
The local mean uses a fixed 5×5 sample window with a default radius of 24 pixels; when local contrast is below 12, it falls back to the global threshold.
No full-frame binary image is retained.

Finder detection starts with the 1:1:3:1:1 ratio in row scans and filters observations using vertical/horizontal cross-checks and a 7×7 pattern check.
Nearby observations are merged into fixed-capacity clusters, ranked by descending observation count, with scan order breaking ties.
When full, a new observation may replace the weakest cluster with a single observation.
Discarding observations is still recorded; if decoding does not succeed, the call returns `FinderCapacity`.

Finder triples are examined in order, choosing the point opposite the longest edge as top-left and determining the orientation.
Finder records store image-horizontal and image-vertical scan pitches separately.
Pattern checks and version estimates use both pitches and account for the difference between rotated scan cross-sections and symbol axes.
Each finder record occupies 20 bytes; detector scratch with the default 32 records occupies 4557 bytes.
Versions are tried in the order estimate, −1, +1, −2, +2, retaining only values in 1–40.
Geometry and finder-pattern checks reject unsuitable candidates before detailed sampling.

| Setting | Default limit |
| --- | ---: |
| Finder records | 32 |
| Total finder triples examined | 4096 |
| Total sampling attempts | 256 |
| Actual input pixel reads | 64,000,000 |
| Row scan step | 1 pixel |

Pixel-read, candidate, and sampling budgets are shared across all threshold and polarity passes.
`PixelBudget`, `CandidateBudget`, and `RetryBudget` explicitly report exhaustion.
`Config.max_pixel_reads` accepts 1–`usize::MAX`; zero is invalid.
Every actual input read consumes the same pixel budget, including initial min/max, repeated coordinates, row scans, cross-checks, finder patterns, alignment search, and matrix sampling.
The next read after the limit returns `PixelBudget` immediately, regardless of retained decode errors or finder overflow.
`Stats.pixel_reads` includes min/max and is accurate on success and failure; validation failures read no pixels.
Low-contrast images consume their min/max reads even though the other counters remain zero.
A decode that completes at exactly the limit remains successful.
The finite default is independent of configured image maxima; callers with larger or harder images can choose a different finite limit.
Measured corpus/benchmark work was 934,433 reads for normal 640×640 V7, 7,964,308 for the 360×360 lighting fixture, and 30,084,436 for the 640×640 undecodable symbol over four passes.
64,000,000 leaves more than twice the largest of these measurements; this is an operational default, not a recognition guarantee.
`Stats.retries` and `max_retries` count matrix-sampling attempts, including samples abandoned for invalid geometry.
A core transpose retry using the same sampled matrix does not consume another sampling attempt.
The first successfully decoded symbol is returned.
There is no contract to enumerate every symbol in an image containing multiple codes.

## Perspective and alignment

For V2 and above, the detector searches around the predicted bottom-right alignment center over ±8 modules on each axis, in 0.5-module steps.
The three finders and alignment center supply four point correspondences for an eight-coefficient homography.
An alignment match is preferred when its score is high and its position is close to the prediction.

After the alignment-based attempt, the detector tries the affine fourth point and eight alternatives formed by changing horizontal and vertical edge lengths by ±10%.
This fourth-point search also runs for V1, allowing perspective correction without an alignment pattern.
The Center strategy reads one pixel at each module center; attempts containing out-of-image or nonfinite coordinates are discarded.

Mirrors are decoded through the core's transpose retry after sampling.
`corners` remain the projections of sampled-grid boundaries `(0,0)`, `(side,0)`, `(side,side)`, and `(0,side)` into the ROI.
Their order is unchanged on mirrored success, so logical top-right and bottom-left after decoding correspond to `corners[3]` and `corners[1]`, respectively.
The public result has no mirrored flag.

Success across all rotation angles or perspective distortions is not guaranteed.
The default configuration has also been tested on rectangular images rendering the same V7 matrix at 4×4, 2×4, and 4×2 pixels per module.
These cases do not guarantee success under arbitrary anisotropic or perspective deformation.
The current fixed corpus covers 17°, 45°, and quarter-turn rotations, strong V1 perspective, V7 alignment-based perspective, inverted polarity, lighting gradients, and mild random noise.
The strong-perspective V1 fixture fails the affine attempt and succeeds through fourth-point search.
The local-threshold fixture fails global thresholds and succeeds on the third pass.
Fixture coordinates, lighting formulas, and random seeds are recorded in `tools/generate_fixtures.py` and `crates/qr-detect/tests/images.rs`.

## Computational bounds and operational limits

A stateless region scan uses at most four passes (a tracker frame can use eight); each local-threshold decision reads 26 pixels (one center read plus 25 window samples, including repeated/clamped coordinates).
Alignment search for one detailed candidate examines at most 1089 positions × 25 modules; one matrix-sampling attempt visits at most 177×177 modules.
A sampled matrix with a normal-orientation content error runs core decoding up to twice.
Transposition requires no additional scratch, but failed input and mirrors can increase CPU time.
The shared pixel budget bounds actual input reads independently of candidate and sampling budgets, even on finderless high-contrast images.
All loops are bounded by input dimensions, independent budgets, or fixed model limits.
The pixel budget does not bound core decoding, geometry calculations, or FFI initialization of caller-provided buffers.
These bounds are not a worst-case execution time guarantee on an MCU.

Positioning uses one homography; local grid correction using every alignment pattern is not implemented.
A difference from a reference algorithm alone is not treated as a standards violation; further local-distortion handling would be a separate feature.
Lens distortion, motion blur, strong reflections, missing finders, modules smaller than one pixel, inadequate quiet zones, and camera-specific noise require separate evaluation.
The default local radius of 24 is not optimal for every module pitch; tune `local_radius` and `row_step` for the imaging system.
Errors such as `FinderCapacity` can guide callers to increase budgets, narrow the ROI, or retry on the next frame.

## Center fallback and frame state

`CenterThenMajority5` uses the same homography and threshold/polarity as the
failed center attempt. The five positions are `(x+0.5,y+0.5)` and its offsets
`(-0.25,0)`, `(0.25,0)`, `(0,-0.25)`, `(0,0.25)` in module coordinates. Three
of five dark votes select a dark module. All five are tested without a majority
short circuit. Every input access, including duplicate pixel positions and the
second center read, consumes the shared pixel budget. A module costs at most
five global reads or 130 local reads in the fallback. Any nonfinite or out-of-image
projection rejects that attempt; projected sample coordinates are never clamped
or substituted from neighboring modules. The existing local threshold window
still clamps its 25 threshold references. Center success adds no retries or reads.

A tracker uses these transitions:

| Previous state | Frame result | New state |
| --- | --- | --- |
| `Searching` | Success | `Tracking` with new hint |
| `Searching` | Complete recognition failure with eligible geometry | `Candidate { misses: 1 }` |
| `Searching` | Complete recognition failure without geometry | `Searching` |
| `Tracking` | Complete recognition failure | `Candidate { misses: 1 }` |
| `Candidate` | Success | `Tracking` with new hint |
| `Candidate` | Second complete recognition failure | `Searching` |
| Any | Validation, capacity, or budget error | Previous state unchanged |

Saved rectangles are tried before the full input image. Ordinary recognition
failure in the ROI triggers a full search during the same call; a ROI equal to
the full image is searched once. `NotFound` and core format/version/content
errors are recognition failures. `FinderCapacity`, candidate/retry/pixel budget
exhaustion, and output/segment/scratch capacity failures leave state unchanged
because the frame did not complete its bounded search. ROI and full-image
min/max scans, finder passes, candidate checks, and sampling attempts share a
single `Stats` and `PixelBudget`, without resetting limits between regions.

Hints use the four projected symbol boundaries' bounding box. Each edge is
expanded by the maximum of 25% of the longest projected edge, an estimate of
four modules, and eight pixels, then rounded outward and clamped to the input
bounds. Degenerate or nonfinite geometry produces no hint. Subregion corners
are translated by the integer origin before returning or saving input coordinates.
An unconfirmed candidate requires validated finder geometry/patterns, a complete
module sample, and a core `ReedSolomon`, `Truncated`, `MalformedPayload`, or
`UnsupportedMode` error after format/version validation. Format/version errors
alone never produce candidates. The first eligible geometry in search order is
retained, but any successful decode takes precedence. New unconfirmed geometry
does not restart the two-frame expiry.

Changed width or height prevents reuse of a saved hint. Stride changes preserve
coordinates. A same-size change of crop origin, scale, or camera coordinate system
requires caller `reset()`. Tracking supports neither camera acquisition, queues,
async ownership, nor enumeration of multiple symbols. Five-point voting may
help isolated center damage, but does not guarantee improvement for blur, very
small modules, strong projection, or missed/decorated finder patterns. Extra
fallback work can leave fewer geometries available under the same work budget.


The sampling choice remains explicit in Rust and in the C tracker API. Existing
stateless C Gray8 APIs use center-then-majority-five by default, following the
paired gate in [performance evaluation](sampling-performance.md). Explicit Center
is available for callers prioritizing long-failure latency or an older tight-budget
contract. Small `max_retries` or `max_pixel_reads` values may leave fewer candidate
projections under the new default, even when ABI-1 struct layouts are unchanged.
