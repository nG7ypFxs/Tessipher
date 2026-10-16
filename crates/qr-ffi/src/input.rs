//! Numeric buffer validation and initialization before borrowing C storage.

use super::{qcore, QrSegment};
use core::{
    mem::{align_of, size_of},
    ptr, slice,
};

#[derive(Clone, Copy)]
pub(super) struct Range {
    start: usize,
    end: usize,
}
impl Range {
    pub(super) const EMPTY: Self = Self { start: 0, end: 0 };
}
pub(super) fn range<T>(p: *const T, n: usize) -> Result<Range, i32> {
    let bytes = n.checked_mul(size_of::<T>()).ok_or(1)?;
    let start = p as usize;
    if bytes > isize::MAX as usize || (bytes != 0 && (p.is_null() || start % align_of::<T>() != 0))
    {
        return Err(1);
    }
    let end = start.checked_add(bytes).ok_or(1)?;
    Ok(Range { start, end })
}
pub(super) fn disjoint(ranges: &[Range]) -> Result<(), i32> {
    for (i, a) in ranges.iter().enumerate() {
        if a.start == a.end {
            continue;
        }
        for b in &ranges[i + 1..] {
            if b.start != b.end && a.start < b.end && b.start < a.end {
                return Err(2);
            }
        }
    }
    Ok(())
}
pub(super) unsafe fn bytes<'a>(p: *const u8, n: usize) -> &'a [u8] {
    if n == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(p, n) }
    }
}
pub(super) unsafe fn bytes_mut<'a>(p: *mut u8, n: usize) -> &'a mut [u8] {
    if n == 0 {
        &mut []
    } else {
        unsafe {
            ptr::write_bytes(p, 0, n);
            slice::from_raw_parts_mut(p, n)
        }
    }
}
pub(super) unsafe fn segments_mut<'a>(p: *mut QrSegment, n: usize) -> &'a mut [qcore::Segment] {
    if n == 0 {
        return &mut [];
    }
    let ptr = p.cast::<qcore::Segment>();
    // C memory can be uninitialized. Establish valid Rust enum values before making a slice.
    for i in 0..n {
        unsafe {
            ptr.add(i).write(qcore::Segment::EMPTY);
        }
    }
    unsafe { slice::from_raw_parts_mut(ptr, n) }
}
