//! The media clock: sleeps and time reads for media tasks.
//!
//! Media code that runs on the socket owner, on a lane poller or as a paced
//! task asks this module for its deadlines and the current time instead of
//! calling Tokio directly, so the executor can serve them from the cheapest
//! source available:
//!
//! - Inside a lane poller's poll with [`MediaExecutorConfig::lane_timer`]
//!   active, [`sleep_until`] files the deadline in the poller's lane clock
//!   (one runtime timer per poller instead of one per call).
//! - Everywhere else it is a plain [`tokio::time::Sleep`].
//!
//! [`now`] and [`now_std`] read the clock media deadlines are measured
//! against: the runtime clock (honouring a paused test clock) and the
//! monotonic system clock. With [`MediaExecutorConfig::lane_pass_clock`]
//! active, inside a lane poller pass they return the pass's single clock
//! reading instead (an earlier reading of the same clock, at most a bounded
//! amount of lane work old), so the futures one pass polls share one
//! `clock_gettime`.
//!
//! [`MediaExecutorConfig::lane_timer`]: crate::executor::MediaExecutorConfig::lane_timer
//! [`MediaExecutorConfig::lane_pass_clock`]: crate::executor::MediaExecutorConfig::lane_pass_clock

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::time::{Instant, Sleep};

pin_project_lite::pin_project! {
    /// A media deadline: a Tokio sleep, or an entry in the lane clock of the
    /// poller hosting the future. Never completes before its instant.
    #[project = MediaSleepProj]
    pub enum MediaSleep {
        /// A runtime timer.
        Legacy { #[pin] sleep: Sleep },
        /// Filed in the lane clock of the poller hosting the future.
        Lane { sleep: crate::lane_clock::LaneSleep },
    }
}

impl MediaSleep {
    /// Move the deadline, like [`Sleep::reset`].
    pub fn reset(self: Pin<&mut Self>, when: Instant) {
        match self.project() {
            MediaSleepProj::Legacy { sleep } => sleep.reset(when),
            MediaSleepProj::Lane { sleep } => sleep.reset(when),
        }
    }

    /// Whether this sleep is filed in a lane clock.
    pub fn is_lane(&self) -> bool {
        matches!(self, Self::Lane { .. })
    }

    /// Hot call state: an address the lane clock prefetches when this sleep
    /// falls due (a runtime timer ignores it).
    pub(crate) fn set_prefetch_hint(self: Pin<&mut Self>, address: usize) {
        if let MediaSleepProj::Lane { sleep } = self.project() {
            sleep.set_prefetch_hint(address);
        }
    }
}

impl Future for MediaSleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        match self.project() {
            MediaSleepProj::Legacy { sleep } => sleep.poll(cx),
            MediaSleepProj::Lane { sleep } => Pin::new(sleep).poll(cx),
        }
    }
}

/// Wait until `when`. Inside a lane poller's poll with the lane timer active
/// the deadline is filed in the poller's lane clock; otherwise this is
/// [`tokio::time::sleep_until`].
pub fn sleep_until(when: Instant) -> MediaSleep {
    if crate::executor::lane_timer_enabled() {
        if let Some(clock) = crate::lane_clock::current() {
            return MediaSleep::Lane {
                sleep: crate::lane_clock::LaneSleep::new(clock, when),
            };
        }
    }
    MediaSleep::Legacy {
        sleep: tokio::time::sleep_until(when),
    }
}

/// Wait for `duration` from [`now`].
pub fn sleep(duration: std::time::Duration) -> MediaSleep {
    sleep_until(now() + duration)
}

/// The instant media deadlines are measured against, on the runtime clock
/// (a paused test clock included).
///
/// With [`MediaExecutorConfig::lane_pass_clock`] active, inside a lane poller
/// pass this is the pass reading: an earlier reading of the same monotonic
/// clock, refreshed after a bounded number of reads and hosted polls,
/// instead of a clock read per call. Everywhere else it is
/// [`tokio::time::Instant::now`].
///
/// Use it where a slightly earlier "now" is safe: deciding whether a
/// deadline has passed (it can only answer "not yet" a little longer, never
/// "yes" too early) and scheduling relative to the media clock. Latency
/// measurements should keep reading the clock.
///
/// [`MediaExecutorConfig::lane_pass_clock`]: crate::executor::MediaExecutorConfig::lane_pass_clock
#[inline]
pub fn now() -> Instant {
    crate::lane_clock::pass_cached_now().unwrap_or_else(Instant::now)
}

/// [`now`] as a [`std::time::Instant`], for state kept in std instants (the
/// socket owner's inline path). Outside a pass-clock pass it is
/// [`std::time::Instant::now`].
#[inline]
pub fn now_std() -> std::time::Instant {
    crate::lane_clock::pass_cached_now().map_or_else(std::time::Instant::now, Instant::into_std)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{override_media_executor_config_on_this_thread, MediaExecutorConfig};
    use std::time::Duration;

    #[tokio::test]
    async fn outside_a_lane_poller_a_media_sleep_is_a_runtime_timer_that_never_fires_early() {
        let _v2 = override_media_executor_config_on_this_thread(MediaExecutorConfig::v2());
        let start = now();
        let mut sleep = Box::pin(sleep_until(start + Duration::from_millis(30)));
        assert!(!sleep.is_lane(), "not inside a poller's poll");
        let when = start + Duration::from_millis(10);
        sleep.as_mut().reset(when);
        sleep.await;
        assert!(Instant::now() >= when);
        sleep_until(now()).await;
    }

    #[tokio::test(start_paused = true)]
    async fn media_time_follows_a_paused_runtime_clock() {
        let start = now();
        sleep(Duration::from_secs(5)).await;
        assert!(now() - start >= Duration::from_secs(5));
    }
}
