macro_rules! abi_schema { () => {
abi_struct! {
    /// ABI-1 runtime detector budgets (five u32 fields); obtain defaults with qr_default_config.
    /// Pixel reads use a finite default or qr_decode_gray_with_budget scalar argument.
    QrConfig => "qr_config" {
    /// Finder-record capacity per pass (3..=128); default 32.
    max_finders: u32,
    /// Shared finder-triple budget (1..=1000000); default 4096.
    max_candidates: u32,
    /// Shared sampling-attempt budget (1..=65536); default 256; excludes core transpose retries.
    max_retries: u32,
    /// Row scan step in pixels (1..=compiled maximum height); default 1.
    row_step: u32,
    /// Half-width of the local threshold sample window in pixels (4..=128); default 24.
    local_radius: u32
}}
abi_struct! {
    /// Compiled image limits, caller-owned storage sizes, and target ABI layout.
    QrMemory => "qr_memory_info" {
    /// ABI version (currently 1).
    abi_version: u32,
    /// Compiled maximum ROI width, in pixels.
    max_width: u32,
    /// Compiled maximum ROI height, in pixels.
    max_height: u32,
    /// Core scratch size, independent of the matrix version.
    core_scratch_bytes: usize,
    /// Detector scratch size for the queried configuration.
    detector_scratch_bytes: usize,
    /// Required alignment of both byte scratch buffers (1).
    scratch_alignment: usize,
    /// Maximum decoded payload byte count (7089).
    max_payload_bytes: usize,
    /// Conservative segment capacity sufficient for any supported payload (5912).
    max_segments: usize,
    /// Size of one segment on this target, in bytes.
    segment_size: usize,
    /// Required segment alignment on this target, in bytes.
    segment_alignment: usize,
    /// Size of a result on this target, in bytes.
    result_size: usize,
    /// Required result alignment on this target, in bytes.
    result_alignment: usize
}}
abi_struct! {
    /// One data or control segment in original bitstream order.
    QrSegment => "qr_segment" {
    /// QR mode indicator: Numeric=1, Alphanumeric=2, SA=3, Byte=4, FNC1 first=5, ECI=7, Kanji=8, FNC1 second=9.
    mode: u32,
    /// Start of the segment's emitted bytes in the payload output.
    offset: usize,
    /// Emitted byte count; control segments have zero length.
    len: usize,
    /// Last ECI assignment, initially 3; controls retain all assignments in order; no transformation is applied.
    eci: u32
}}
abi_struct! {
    /// Successful decoding metadata; only payload_len bytes and segment_count entries are valid.
    QrResult => "qr_result" {
    /// QR Model 2 version (1..=40).
    version: u32,
    /// Error-correction level: 0=L, 1=M, 2=Q, 3=H.
    ec_level: u32,
    /// Mask pattern (0..=7).
    mask: u32,
    /// Valid prefix length of the payload output, in bytes.
    payload_len: usize,
    /// Valid segment count, including control and empty data segments.
    segment_count: usize,
    /// Total corrected codewords across all Reed-Solomon blocks.
    corrected_codewords: u32,
    /// Selected format BCH Hamming distance (0..=3).
    format_distance: u32,
    /// Selected version BCH Hamming distance (0..=3); zero for V1-6.
    version_distance: u32,
    /// FNC1 header: 0=none, 1=first position, 2=second position.
    fnc1: u32,
    /// Raw second-position application indicator; zero unless fnc1=2.
    fnc1_application_indicator: u32,
    /// One when Structured Append metadata is present, otherwise zero.
    sa_present: u32,
    /// Zero-based symbol index; meaningful only when sa_present=1.
    sa_index: u32,
    /// Symbol count (1..=16); meaningful only when sa_present=1.
    sa_total: u32,
    /// Original parity byte; meaningful only when sa_present=1; no reassembly is performed.
    sa_parity: u32,
    /// One for an inverted-polarity detection; zero for matrix decoding.
    inverted: u32,
    /// One for local-threshold detection; zero for matrix decoding.
    local_threshold: u32,
    /// Finder triples examined across passes; zero for matrix decoding.
    candidates: usize,
    /// Sampling attempts, including geometric rejection; excludes core transpose retries; zero for matrix decoding.
    retries: usize,
    /// ROI-relative x of sampled-grid boundary (0,0); zero for matrix decoding.
    corner0_x: f32,
    /// ROI-relative y of sampled-grid boundary (0,0); zero for matrix decoding.
    corner0_y: f32,
    /// ROI-relative x of sampled-grid boundary (side,0); zero for matrix decoding.
    corner1_x: f32,
    /// ROI-relative y of sampled-grid boundary (side,0); zero for matrix decoding.
    corner1_y: f32,
    /// ROI-relative x of sampled-grid boundary (side,side); zero for matrix decoding.
    corner2_x: f32,
    /// ROI-relative y of sampled-grid boundary (side,side); zero for matrix decoding.
    corner2_y: f32,
    /// ROI-relative x of sampled-grid boundary (0,side); zero for matrix decoding.
    corner3_x: f32,
    /// ROI-relative y of sampled-grid boundary (0,side); zero for matrix decoding.
    corner3_y: f32
}}
abi_struct! {
    /// Caller-owned initialized integer tracker; all-zero storage means Searching.
    /// No image/payload/pointer is retained. Do not decode uninitialized storage.
    QrTracker => "qr_tracker" {
    /// Searching=0, Candidate=1, Tracking=2; all other tags are invalid.
    tag: u32,
    /// Exactly 1 for Candidate, 0 for Searching/Tracking.
    misses: u32,
    /// Saved ROI left coordinate in input-image pixels; zero when Searching.
    roi_x: usize,
    /// Saved ROI top coordinate in input-image pixels; zero when Searching.
    roi_y: usize,
    /// Nonzero saved ROI width; zero when Searching.
    roi_width: usize,
    /// Nonzero saved ROI height; zero when Searching.
    roi_height: usize,
    /// Saved input width (1..32768); zero when Searching.
    image_width: usize,
    /// Saved input height (1..32768); zero when Searching.
    image_height: usize
}}
abi_struct! {
    /// Actual decode-call work, written on success or failure after decoder invocation.
    /// Optional output remains untouched when arguments are rejected before decoding.
    QrStats => "qr_stats" {
    /// Actual input reads, including repeats; never exceeds the supplied limit.
    pixel_reads: usize,
    /// Finder rows scanned across all regions/passes.
    rows: usize,
    /// Finder run observations before detailed pattern validation.
    finder_observations: usize,
    /// Finder triples examined across all regions/passes.
    candidates: usize,
    /// Sampling attempts, including fallback; core transpose adds no attempt.
    retries: usize,
    /// Threshold/polarity passes (up to 4 stateless, 8 tracked).
    passes: usize
}}
abi_struct! {
    /// Target layout query for the new tracker and stats; ABI-1 memory layout is unchanged.
    QrTrackerMemory => "qr_tracker_memory_info" {
    /// Size of one integer tracker in bytes.
    tracker_size: usize,
    /// Required tracker alignment in bytes.
    tracker_alignment: usize,
    /// Size of one stats structure in bytes.
    stats_size: usize,
    /// Required stats alignment in bytes.
    stats_alignment: usize
}}
}; }
