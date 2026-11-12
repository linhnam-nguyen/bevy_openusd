use std::sync::atomic::{AtomicU8, Ordering};

use anyhow::{Result, bail};

static FAILURE_POINT: AtomicU8 = AtomicU8::new(0);

pub(super) fn set(point: u8) {
    FAILURE_POINT.store(point, Ordering::SeqCst);
}

pub(super) fn maybe(point: u8) -> Result<()> {
    if FAILURE_POINT
        .compare_exchange(point, 0, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        bail!("injected Project migration failure at point {point}");
    }
    Ok(())
}
