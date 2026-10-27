//! Measure live allocator bytes. This binary has one test so unrelated
//! tests cannot perturb the measurement.
use malt_gateway::rate_limit::RateLimiter;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Duration;

struct MeasuredAllocator;
static LIVE_BYTES: AtomicIsize = AtomicIsize::new(0);

// SAFETY: allocations are delegated unchanged to System with atomic accounting.
unsafe impl GlobalAlloc for MeasuredAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller supplies GlobalAlloc's valid allocation layout.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            LIVE_BYTES.fetch_add(layout.size() as isize, Ordering::SeqCst);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE_BYTES.fetch_sub(layout.size() as isize, Ordering::SeqCst);
        // SAFETY: pointer and layout are passed through from the allocator caller.
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: MeasuredAllocator = MeasuredAllocator;

#[test]
fn expired_callers_release_live_heap_bytes_on_normal_access() {
    let window = Duration::from_secs(1);
    let limiter = RateLimiter::with_limits(100, 100_000, window).unwrap();
    let baseline = LIVE_BYTES.load(Ordering::SeqCst);
    for index in 0..20_000 {
        assert!(limiter.check(&format!("caller-{index:069}")));
    }
    let peak = LIVE_BYTES.load(Ordering::SeqCst) - baseline;
    assert!(
        peak > 1_000_000,
        "fixture must allocate meaningful state: {peak}"
    );
    std::thread::sleep(window);
    assert!(limiter.check("next-caller"));
    let retained = LIVE_BYTES.load(Ordering::SeqCst) - baseline;
    assert!(
        retained < 16_384,
        "expired state retained {retained} bytes from {peak}"
    );
    eprintln!("limiter live heap: peak_delta={peak} retained_delta={retained}");
}
