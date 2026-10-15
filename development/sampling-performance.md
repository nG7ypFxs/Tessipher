# Sampling and tracking benchmarks

Use these benchmarks when changing detection, sampling, tracking, or FFI buffer
handling. Run from the repository root with Rust/Cargo, Python 3, and a host C
compiler:

```sh
cargo run -p tessipher --example sampling_bench --release --locked
python3 tools/c_sampling_bench.py
```

The C script supports macOS and Linux and respects `CARGO_TARGET_DIR`. Both
benchmarks use release builds, fixed synthetic inputs, three warmups, and 15
paired samples with alternating strategy order. Input generation, allocation,
and printing are outside the timed calls. The C measurements include FFI buffer
initialization and compare small output capacities with the queried maximum
capacities.

## Reading the output

Compare Center with CenterThenMajority5 per input, using both elapsed time and
pixel-read, candidate, retry, and pass counters. The paired ratio is the median
of individual majority/center time ratios. Tracker sequences include movement,
disappearance, false candidates, recovery, and resizing; inspect those failures
alongside stable frames.

The current benchmark checks require:

1. Center successes to remain successful with five-point fallback.
2. Work counters to match when Center succeeds on its first sampling attempt.
3. The paired time ratio for Center-success inputs to be at most 1.10.
4. The `undecodable-v7` time and pixel-read ratios to be at most 1.50.
5. Core content failures not to become candidate, retry, or pixel-budget errors.

Check `DEFAULT_GATE_PASSED=true` in Rust and `C_DEFAULT_GATE_PASSED=1` in C,
including the individual per-case checks. A successful process exit alone does
not establish that these printed checks passed. These thresholds are development
regression criteria, not recognition or MCU timing guarantees. Repeat noisy
measurements on an otherwise idle host before drawing timing conclusions.

When comparing revisions, keep the compiler, target, release settings, corpus,
budgets, and output capacities identical. Record those settings and the commit
IDs with the report. Save local output under `target/` rather than treating a
single host run as current library documentation.

## Interpreting the tradeoffs

Five-point fallback runs only after a center sample fails content decoding. It
can recover damaged module centers, but adds reads and retries on failed inputs;
under finite budgets, later candidates may not be reached. Rust defaults to
Center; C stateless Gray8 entry points default to CenterThenMajority5. Explicit
Center is available when long-failure latency or budget usage matters more.

Tracking can reduce repeated searches for stable symbols. An unsuccessful ROI
search also performs full-image recovery in the same frame, so tracking does not
always reduce work. C calls initialize caller-provided buffers; unnecessarily
large payload and segment capacities can dominate short calls.

Host results do not establish physical MCU cycles, stack usage, SDRAM traffic,
camera frame rates, or recognition accuracy. Use [target measurements](../docs/ffi.md#measurements-on-an-mcu)
and [camera measurements](mac-camera-benchmark.md) for the relevant workload.
