//! Reads from a mapping another process writes while it is mapped.
//!
//! QEMU and the guest write the LCD mappings at any time, so every read goes
//! through a raw pointer as an atomic load. Through a `&[u8]`, the compiler
//! treats the bytes as immutable for the borrow's lifetime, and an optimised
//! build folds or hoists the reads of a polled counter.

use std::marker::PhantomData;
use std::sync::atomic::{fence, AtomicU16, AtomicU32, Ordering};

/// A view of `len` bytes another process writes.
#[derive(Clone, Copy)]
pub(crate) struct View<'a> {
    base: *const u8,
    len: usize,
    _mapping: PhantomData<&'a [u8]>,
}

impl<'a> View<'a> {
    /// `bytes` must start on a 4-byte boundary, as a mapping does.
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        assert!(
            (bytes.as_ptr() as usize).is_multiple_of(4),
            "unaligned mapping"
        );
        Self {
            base: bytes.as_ptr(),
            len: bytes.len(),
            _mapping: PhantomData,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// The little-endian `u32` at `off`, which must be 4-byte aligned.
    pub(crate) fn u32_at(&self, off: usize, order: Ordering) -> u32 {
        assert!(off.is_multiple_of(4) && off + 4 <= self.len, "u32 at {off}");
        // SAFETY: in bounds and aligned (checked above); the writer stores
        // whole words, and an atomic load tolerates a concurrent store.
        let word = unsafe { &*(self.base.add(off) as *const AtomicU32) };
        u32::from_le(word.load(order))
    }

    /// The little-endian `u16` at `off`, which must be 2-byte aligned.
    pub(crate) fn u16_at(&self, off: usize) -> u16 {
        assert!(off.is_multiple_of(2) && off + 2 <= self.len, "u16 at {off}");
        // SAFETY: as for `u32_at`.
        let half = unsafe { &*(self.base.add(off) as *const AtomicU16) };
        u16::from_le(half.load(Ordering::Relaxed))
    }

    /// Copy `dst.len()` bytes from `off`, then fence so a following counter
    /// read cannot move before the copy (the seqlock's second read).
    pub(crate) fn copy_to(&self, off: usize, dst: &mut [u8]) {
        assert!(off + dst.len() <= self.len, "copy at {off}");
        // SAFETY: in bounds (checked above); `dst` is a separate allocation.
        // A torn copy is detected by the caller's seqlock.
        unsafe { std::ptr::copy_nonoverlapping(self.base.add(off), dst.as_mut_ptr(), dst.len()) };
        fence(Ordering::Acquire);
    }
}
