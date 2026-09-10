//! Host-configurable tooltip timing.
//!
//! The grid's own hover chips (truncated pivot chips, the sidebar's save
//! button) use gpui's tooltip machinery, which shows after 500ms. An app that
//! has picked a different dwell time for its own tooltips would otherwise have
//! the table disagree with everything around it, so the delay is process-global
//! and settable by the host.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// gpui's own default, and the crate's until a host says otherwise.
const DEFAULT_SHOW_DELAY_MS: u64 = 500;

static SHOW_DELAY_MS: AtomicU64 = AtomicU64::new(DEFAULT_SHOW_DELAY_MS);

/// Set how long the pointer must rest on an element before the table shows its
/// tooltip. Applies to every table in the process, from the next frame on.
pub fn set_tooltip_show_delay(delay: Duration) {
    SHOW_DELAY_MS.store(
        delay.as_millis().min(u64::MAX as u128) as u64,
        Ordering::Relaxed,
    );
}

/// The current tooltip dwell time — [`set_tooltip_show_delay`]'s value, or
/// 500ms if the host never set one.
#[must_use]
pub fn tooltip_show_delay() -> Duration {
    Duration::from_millis(SHOW_DELAY_MS.load(Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_defaults_to_gpuis_and_follows_the_host() {
        // Process-global: capture and restore so parallel tests are unaffected.
        let original = tooltip_show_delay();
        set_tooltip_show_delay(Duration::from_millis(DEFAULT_SHOW_DELAY_MS));
        assert_eq!(tooltip_show_delay(), Duration::from_millis(500));

        set_tooltip_show_delay(Duration::from_secs(2));
        assert_eq!(tooltip_show_delay(), Duration::from_secs(2));

        set_tooltip_show_delay(original);
    }
}
