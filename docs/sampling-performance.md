# Sampling default and C tracking performance

The C stateless Gray8 default is **CenterThenMajority5**. Existing Rust
`decode`/`decode_with_stats` remain Center, and C callers can choose
`QR_SAMPLING_CENTER` through the explicit sampling API. Tracking is always
explicit caller-owned state. The change preserves ABI-1 config/result/memory
layouts and existing symbols; behavior after a failed center attempt changes.

## Decision gate and method

The gate was fixed in the benchmark source before measurement:

1. No previously successful default-budget corpus input may lose decoding success.
2. Normal first-center-success work counters must remain identical.
3. Successful-input paired median time growth must be at most 10%.
4. Representative long-failure paired time and read growth must be at most 50%.
5. An old core-content failure must not become candidate/retry/pixel-budget exhaustion.

These thresholds are an operational choice based on host variation and the
failure-only design, not standards requirements or MCU worst-case guarantees.
The full public corpus and separate private photo batch passed. Independent
repeat batches confirmed the decision, and the C end-to-end gate passed with
both small and conservative maximum capacities. No result was averaged only
across improved cases. Perspective failures preceding success and entirely
failed frames are shown separately.

All runs used release builds on aarch64 macOS, fixed seed `0x12345678`, default
Config/pixel budgets, three warmups per method followed by 15 samples. Pair
execution order alternates every sample. Rust decode results go through
`black_box`; C calls cross the static-library ABI. Input creation/loading,
allocation, assertions, and printing are outside measured intervals. Every
public success asserts `TEST`; successful policies internally compare payload,
segments and symbol metadata. Private bytes are never printed or saved.
Counters and outcomes are deterministic and checked between samples.

Reproduce:

```sh
cargo run -p qr --example sampling_bench --release
python3 tools/c_sampling_bench.py
```

The C tool respects `CARGO_TARGET_DIR`, including root-relative paths, and builds
and links the same selected target directory. A fresh `/tmp` target directory
was also built and executed. The raw public results contain all outcomes,
work counters, min/median/test-set-max times and frame measurements:
[Rust record](sampling-benchmark-rust.txt), [C record](sampling-benchmark-c.txt).
Private photo inputs/results remain outside the repository.

## Rust stateless corpus

Times are center / majority medians in μs; the ratio is the median of individual
paired majority/center times, rather than a ratio of aggregate medians.

| Case | Outcome | Median μs (Center / Majority) | Paired ratio | Pixel reads (Center / Majority) |
| --- | --- | ---: | ---: | ---: |
| normal-v1 | OK → OK | 1,562.4 / 1,563.0 | 0.9958 | 837,804 / 837,804 |
| normal-v7 | OK → OK | 1,562.5 / 1,560.2 | 0.9965 | 934,433 / 934,433 |
| normal-v32 | OK → OK | 2,444.0 / 2,434.7 | 0.9970 | 687,064 / 687,064 |
| normal-v40 | OK → OK | 3,629.4 / 3,626.9 | 0.9957 | 1,051,587 / 1,051,587 |
| mirror-v1 | OK → OK | 1,104.9 / 1,104.8 | 0.9991 | 838,084 / 838,084 |
| rotation45 | OK → OK | 683.5 / 685.1 | 1.0042 | 338,497 / 338,497 |
| perspective | OK → OK | 502.6 / 533.6 | 1.0657 | 298,439 / 302,849 |
| lighting | OK → OK | 8,572.2 / 8,470.4 | 0.9941 | 7,964,308 / 7,964,308 |
| low-resolution | OK → OK | 23.5 / 23.5 | 0.9983 | 9,619 / 9,619 |
| anisotropic | OK → OK | 177.9 / 177.9 | 0.9993 | 108,537 / 108,537 |
| fixed-noise | CandidateBudget → CandidateBudget | 7,236.9 / 7,226.5 | 1.0066 | 1,524,660 / 1,524,660 |
| no-qr | NotFound → NotFound | 39.0 / 39.0 | 1.0000 | 409,600 / 409,600 |
| undecodable-v7 | Version → Version | 34,197.7 / 46,803.7 | 1.3692 | 30,084,436 / 38,328,886 |
| center-damage | ReedSolomon → OK | 4,171.0 / 245.5 | 0.0590 | 3,903,291 / 135,976 |

First-center successes, including mirror, rotation, local threshold, low
resolution and anisotropic modules, have identical work. The strong-perspective
input succeeds after earlier center failures: majority tries extra fallback
samples, increasing retries from 3 to 5 and reads from 298,439 to 302,849. Its
paired time ratio is 1.0657; it is distinct from the first-center-success path.

The undecodable V7 incurs **36.9% more paired time and 27.4% more reads**, with
60 → 120 retries. Noise and no-QR work stay unchanged. The center-damage frame
changes from ReedSolomon failure to success at about 5.9% of the original time.
Tight retry/read budgets can still change which geometries get examined or the
final error. Callers prioritizing long-failure latency or previous budget behavior
should choose explicit Center. Default-budget corpus results do not establish
compatibility under every smaller budget or guarantee the same first symbol in
images with multiple candidates.

## Frame sequences and C capacity costs

Each of 15 finite sequences resets only at its beginning. Eleven frames cover
initial/stable/small move/ROI-outside move/disappearance twice/reappearance/
false candidate twice/recovery/resize. Frame and total distributions are recorded.
The deliberately failure-heavy sequence's majority/center total ratio is 1.1027
stateless and 1.1782 tracked in Rust. False candidates cause ROI plus full-image
failure work (eight passes), so this sequence does not show universal tracking
speedup. Candidate replacement does not restart its two-frame expiry.

The C benchmark includes all FFI scratch/payload/segment initialization. It uses
exact queried scratch lengths and compares payload32/segments8 against
payload7089/segments5912 (189,184 segment bytes on this host). Both capacity
modes pass the gate: normal V1/V7 paired ratios are 0.9985–1.0000 and long-failure
ratios are 1.3712–1.3714, with the same deterministic read increase. The mixed
sequence's paired ratios are about 1.095–1.101 stateless and 1.178 tracked.

For stable C frames, first full search uses 246,372 reads; the next ROI uses
38,796 reads (84.3% fewer), reducing approximately 335 μs to 81–83 μs in the
measured batches. Small movement retains the ROI; an outside movement performs
same-frame full recovery with combined work. One-pixel NotFound calls isolate
an initialization floor including validation and one min/max read. Averaging
128 calls per sample avoids timer quantization: approximately 0.16 μs with small
capacities and 2.75 μs with maximum capacities. This is not a pure buffer-clear
measurement, and maximum-capacity initialization can matter more for smaller
or faster inputs on different targets.

C tracker storage is 56 bytes/alignment8 and stats48/alignment8 on this host.
Neither retains image-sized data, payloads or pointers. Query target layouts;
do not assume host sizes on Cortex-M.

Private photo 2 at 384 pixels changes from failure to success with a paired
ratio around 0.0598. Stable tracked majority uses 354,854 reads initially and
239,500 on the next frame. The Center tracker still fails there and spends
18,974,108 reads on its second ROI-plus-full search. Photo 1/2 previously
successful ROIs retain first-center work; photo 3 remains NotFound with zero
sampling retries, so this change does not improve its missing finder geometry.
No photos, private payloads, URLs or WiFi credentials were added to these records.

Host timings vary. No MCU cycle/stack measurements, camera frame-rate guarantees,
or recognition guarantee for blur/low-resolution/decorated finders follows from
this corpus. CPU work beyond pixel reads and caller-buffer initialization must
still be measured on the target board.
