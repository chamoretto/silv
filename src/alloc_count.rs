//! Counting allocator used by `--memory-report` to measure exact heap usage.
//!
//! The allocator is installed in the binary and forwards every request to the
//! system allocator. While disabled the only cost is one relaxed atomic load
//! per allocation. The counters measure user-requested bytes, not the heap
//! management headers added by the allocator underneath.

use std::alloc::GlobalAlloc;
use std::alloc::Layout;
use std::alloc::System;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

/// Allocation size classes: class `k` covers requests of `2^(k-1) + 1 ..= 2^k`
/// bytes, class 0 covers a single byte. Index by [`size_class`].
pub const SIZE_CLASSES: usize = 65;

const fn size_class(size: usize) -> usize {
    let pow = (size as u64).next_power_of_two();
    64 - pow.leading_zeros() as usize
}

/// A point-in-time reading of the counters.
#[derive(Clone, Debug)]
pub struct HeapStats {
    pub live: u64,
    pub peak_live: u64,
    pub allocs: u64,
    pub total: u64,
    pub live_by_class: [u64; SIZE_CLASSES],
    pub allocs_by_class: [u64; SIZE_CLASSES],
}

impl Default for HeapStats {
    fn default() -> Self {
        Self {
            live: 0,
            peak_live: 0,
            allocs: 0,
            total: 0,
            live_by_class: [0; SIZE_CLASSES],
            allocs_by_class: [0; SIZE_CLASSES],
        }
    }
}

static ENABLED: AtomicBool = AtomicBool::new(false);
static LIVE: AtomicU64 = AtomicU64::new(0);
static PEAK_LIVE: AtomicU64 = AtomicU64::new(0);
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static TOTAL: AtomicU64 = AtomicU64::new(0);
static LIVE_CLASS: [AtomicU64; SIZE_CLASSES] = [const { AtomicU64::new(0) }; SIZE_CLASSES];
static ALLOCS_CLASS: [AtomicU64; SIZE_CLASSES] = [const { AtomicU64::new(0) }; SIZE_CLASSES];

/// A system allocator that counts live bytes, allocations and size classes
/// while [`enable`] is active.
pub struct Counting;

/// Starts counting from zero. Counting is global to the process and racy under
/// multithreaded allocation, so peaks are approximate: the live value read at
/// a quiescent point (no parse in flight) is exact.
pub fn enable() {
    LIVE.store(0, Ordering::Relaxed);
    PEAK_LIVE.store(0, Ordering::Relaxed);
    ALLOCS.store(0, Ordering::Relaxed);
    TOTAL.store(0, Ordering::Relaxed);
    for class in LIVE_CLASS.iter().zip(ALLOCS_CLASS.iter()) {
        class.0.store(0, Ordering::Relaxed);
        class.1.store(0, Ordering::Relaxed);
    }
    ENABLED.store(true, Ordering::Relaxed);
}

pub fn disable() {
    ENABLED.store(false, Ordering::Relaxed);
}

pub fn snapshot() -> HeapStats {
    let live = LIVE.load(Ordering::Relaxed);
    HeapStats {
        peak_live: live.max(PEAK_LIVE.load(Ordering::Relaxed)),
        live,
        allocs: ALLOCS.load(Ordering::Relaxed),
        total: TOTAL.load(Ordering::Relaxed),
        live_by_class: class_array(&LIVE_CLASS),
        allocs_by_class: class_array(&ALLOCS_CLASS),
    }
}

fn class_array(counters: &[AtomicU64; SIZE_CLASSES]) -> [u64; SIZE_CLASSES] {
    let mut out = [0u64; SIZE_CLASSES];
    for (index, counter) in counters.iter().enumerate() {
        out[index] = counter.load(Ordering::Relaxed);
    }
    out
}

impl Counting {
    fn on_alloc(&self, layout: &Layout) {
        let size = layout.size() as u64;
        let live = LIVE.fetch_add(size, Ordering::Relaxed) + size;
        PEAK_LIVE.fetch_max(live, Ordering::Relaxed);
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        TOTAL.fetch_add(size, Ordering::Relaxed);
        let class = size_class(layout.size());
        LIVE_CLASS[class].fetch_add(size, Ordering::Relaxed);
        ALLOCS_CLASS[class].fetch_add(1, Ordering::Relaxed);
    }

    fn on_dealloc(&self, layout: &Layout) {
        LIVE.fetch_sub(layout.size() as u64, Ordering::Relaxed);
        LIVE_CLASS[size_class(layout.size())].fetch_sub(layout.size() as u64, Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ENABLED.load(Ordering::Relaxed) {
            self.on_alloc(&layout);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ENABLED.load(Ordering::Relaxed) {
            self.on_dealloc(&layout);
        }
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ENABLED.load(Ordering::Relaxed) {
            self.on_alloc(&Layout::from_size_align(new_size, layout.align()).unwrap_or(layout));
            self.on_dealloc(&layout);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if ENABLED.load(Ordering::Relaxed) {
            self.on_alloc(&layout);
        }
        unsafe { System.alloc_zeroed(layout) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_classes_match_power_of_two() {
        assert_eq!(size_class(1), 1);
        assert_eq!(size_class(2), 2);
        assert_eq!(size_class(3), 3);
        assert_eq!(size_class(5), 4);
        assert_eq!(size_class(4096), 13);
    }

    #[test]
    fn snapshots_are_instantiated() {
        let stats = HeapStats::default();
        assert_eq!(stats.live, 0);
        assert_eq!(stats.live_by_class.len(), SIZE_CLASSES);
    }
}
