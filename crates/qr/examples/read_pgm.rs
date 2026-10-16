//! Host-only example. The library itself neither allocates nor reads files.
use tessipher::{core, detect};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: read_pgm FILE.pgm (P5, maxval255, no comments)")?;
    let data = std::fs::read(path)?;
    let mut split = data.splitn(4, |b| *b == b'\n');
    if split.next() != Some(b"P5".as_slice()) {
        return Err("P5 required".into());
    }
    let dims = std::str::from_utf8(split.next().ok_or("dimensions")?)?;
    let mut dims = dims.split_whitespace();
    let w = dims.next().ok_or("width")?.parse()?;
    let h = dims.next().ok_or("height")?.parse()?;
    if split.next() != Some(b"255".as_slice()) {
        return Err("maxval255 required".into());
    }
    let pixels = split.next().ok_or("pixels")?;
    let cfg = detect::DefaultConfig::default();
    let mut ds = vec![0; cfg.scratch_bytes().map_err(|e| format!("{e:?}"))?];
    let mut cs = vec![0; core::SCRATCH_BYTES];
    let mut out = vec![0; core::MAX_PAYLOAD_BYTES];
    let mut seg = vec![core::Segment::EMPTY; core::MAX_SEGMENTS];
    let img = detect::GrayImage::new(pixels, w, h, w).map_err(|e| format!("{e:?}"))?;
    let result = detect::decode(img, &cfg, &mut ds, &mut cs, &mut out, &mut seg)
        .map_err(|e| format!("{e:?}"))?;
    println!(
        "{result:?}\npayload bytes: {:?}",
        &out[..result.decoded.payload_len]
    );
    Ok(())
}
