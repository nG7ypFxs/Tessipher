macro_rules! abi_schema { () => {
abi_struct! { QrConfig => "qr_config" {
    max_finders: u32, max_candidates: u32, max_retries: u32, row_step: u32, local_radius: u32
}}
abi_struct! { QrMemory => "qr_memory_info" {
    abi_version: u32, max_width: u32, max_height: u32,
    core_scratch_bytes: usize, detector_scratch_bytes: usize, scratch_alignment: usize,
    max_payload_bytes: usize, max_segments: usize, segment_size: usize, segment_alignment: usize,
    result_size: usize, result_alignment: usize
}}
abi_struct! { QrSegment => "qr_segment" { mode: u32, offset: usize, len: usize, eci: u32 }}
abi_struct! { QrResult => "qr_result" {
    version: u32, ec_level: u32, mask: u32, payload_len: usize, segment_count: usize,
    corrected_codewords: u32, format_distance: u32, version_distance: u32,
    fnc1: u32, fnc1_application_indicator: u32, sa_present: u32, sa_index: u32, sa_total: u32, sa_parity: u32,
    inverted: u32, local_threshold: u32, candidates: usize, retries: usize,
    corner0_x: f32, corner0_y: f32, corner1_x: f32, corner1_y: f32,
    corner2_x: f32, corner2_y: f32, corner3_x: f32, corner3_y: f32
}}
}; }
