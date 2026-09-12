use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(1);

#[inline]
pub fn next() -> u64 {
    let mut id = NEXT.fetch_add(1, Ordering::SeqCst);
    if id == 0 {
        NEXT.store(1, Ordering::SeqCst);
        id = NEXT.fetch_add(1, Ordering::SeqCst);
    }
    if id == 0 { 1 } else { id }
}

#[inline]
pub fn seed_from_max(max_seen: u64) {
    let next = max_seen.saturating_add(1).max(1);
    NEXT.fetch_max(next, Ordering::SeqCst);
}
