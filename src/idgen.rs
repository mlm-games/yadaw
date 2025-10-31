use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(1);

#[inline]
pub fn next() -> u64 {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    if id == 0 {
        NEXT.fetch_add(1, Ordering::Relaxed)
    } else {
        id
    }
}

#[inline]
pub fn seed_from_max(max_seen: u64) {
    let next = max_seen.saturating_add(1).max(1);
    NEXT.store(next, Ordering::Relaxed);
}
