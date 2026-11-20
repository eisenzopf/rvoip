//! Per-thread reuse of PCM sample buffers on the media path.
//!
//! With [`MediaExecutorConfig::pcm_pool`] active (it is part of
//! [`MediaExecutorConfig::v2`]), the receive path takes the sample buffers of
//! decoded and concealed audio frames from a small pool owned by the current
//! thread, and gives back the buffers of frames it discards (late, duplicate,
//! evicted, overflowing a queue, refused by a full consumer). A consumer of
//! decoded frames that is done with one may hand its buffer back with
//! [`recycle_pcm_buffer`]. In a steady call on a media lane, decode then
//! reuses the same few buffers instead of allocating one per 20 ms frame.
//!
//! The pool is per thread, so it takes no lock; a buffer returned on another
//! thread simply joins that thread's pool. It is bounded
//! ([`PCM_POOL_CAPACITY`] buffers of at most [`PCM_POOL_MAX_SAMPLES`]
//! samples each). With the switch off (always under v1) every function here
//! behaves exactly like the allocation it replaces and the pool stays empty.
//!
//! [`MediaExecutorConfig::pcm_pool`]: rvoip_rtp_core::executor::MediaExecutorConfig::pcm_pool
//! [`MediaExecutorConfig::v2`]: rvoip_rtp_core::executor::MediaExecutorConfig::v2

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicU64, Ordering};

use rvoip_rtp_core::executor::pcm_pool_enabled;

/// Buffers one thread keeps for reuse. A 20 ms G.711 frame needs 160
/// samples, and a media lane holds only a few frames in flight per call
/// (reorder hold, pacing queue, the consumer's playout), so a small pool
/// covers a lane's steady state.
pub const PCM_POOL_CAPACITY: usize = 32;

/// Largest buffer (in samples of capacity) the pool keeps; larger buffers
/// are freed when returned.
pub const PCM_POOL_MAX_SAMPLES: usize = 4096;

/// How many reuses a thread counts locally before publishing them.
const FLUSH_EVERY: u64 = 64;

static REUSES: AtomicU64 = AtomicU64::new(0);
static MISSES: AtomicU64 = AtomicU64::new(0);

/// Reuses counted on one thread and not yet published; published when the
/// batch fills and when the thread exits.
struct PendingReuses(Cell<u64>);

impl PendingReuses {
    fn add_one(&self) {
        let value = self.0.get() + 1;
        if value >= FLUSH_EVERY {
            self.0.set(0);
            REUSES.fetch_add(value, Ordering::Relaxed);
        } else {
            self.0.set(value);
        }
    }
}

impl Drop for PendingReuses {
    fn drop(&mut self) {
        let value = self.0.get();
        if value > 0 {
            REUSES.fetch_add(value, Ordering::Relaxed);
        }
    }
}

thread_local! {
    static POOL: RefCell<Vec<Vec<i16>>> = const { RefCell::new(Vec::new()) };
    static PENDING_REUSES: PendingReuses = const { PendingReuses(Cell::new(0)) };
    static THREAD_MISSES: Cell<u64> = const { Cell::new(0) };
}

/// PCM pool totals for this process since start. Each field is individually
/// atomic; the pair is not a transaction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PcmPoolCounters {
    /// Buffers taken from a thread's pool instead of being allocated.
    /// Published in batches per thread, so the most recent few may not be
    /// counted yet.
    pub reuses: u64,
    /// Buffers allocated while the pool was enabled because the thread's
    /// pool was empty. Flat after warm-up in a steady state.
    pub misses: u64,
}

/// The PCM pool totals since start.
pub fn pcm_pool_counters() -> PcmPoolCounters {
    PcmPoolCounters {
        reuses: REUSES.load(Ordering::Relaxed),
        misses: MISSES.load(Ordering::Relaxed),
    }
}

/// An empty PCM buffer with room for `samples`: one from this thread's pool
/// when the pool is enabled and holds one, otherwise freshly allocated
/// (`Vec::with_capacity(samples)`).
pub fn take_pcm_buffer(samples: usize) -> Vec<i16> {
    if pcm_pool_enabled() {
        if let Some(mut buffer) = POOL.with(|pool| pool.borrow_mut().pop()) {
            buffer.clear();
            buffer.reserve(samples);
            PENDING_REUSES.with(PendingReuses::add_one);
            return buffer;
        }
        // An allocation the pool could not avoid; one atomic next to a
        // malloc is noise.
        MISSES.fetch_add(1, Ordering::Relaxed);
        THREAD_MISSES.with(|misses| misses.set(misses.get() + 1));
    }
    Vec::with_capacity(samples)
}

/// Hand a spent PCM buffer back to this thread's pool. It is dropped
/// instead when the pool is disabled or full, or the buffer is empty or
/// larger than [`PCM_POOL_MAX_SAMPLES`].
pub fn recycle_pcm_buffer(buffer: Vec<i16>) {
    if buffer.capacity() == 0 || buffer.capacity() > PCM_POOL_MAX_SAMPLES || !pcm_pool_enabled() {
        return;
    }
    POOL.with(|pool| {
        let mut pool = pool.borrow_mut();
        if pool.len() < PCM_POOL_CAPACITY {
            pool.push(buffer);
        }
    });
}

/// `vec![0; samples]` (a concealment frame), from the pool when enabled.
pub(crate) fn take_zeroed_pcm_buffer(samples: usize) -> Vec<i16> {
    if pcm_pool_enabled() {
        let mut buffer = take_pcm_buffer(samples);
        buffer.resize(samples, 0);
        return buffer;
    }
    vec![0; samples]
}

/// `samples.to_vec()` (a repetition of the last frame), from the pool when
/// enabled.
pub(crate) fn take_pcm_copy(samples: &[i16]) -> Vec<i16> {
    if pcm_pool_enabled() {
        let mut buffer = take_pcm_buffer(samples.len());
        buffer.extend_from_slice(samples);
        return buffer;
    }
    samples.to_vec()
}

/// Test seam: the number of buffers in this thread's pool.
#[doc(hidden)]
pub fn pcm_pool_len_on_this_thread() -> usize {
    POOL.with(|pool| pool.borrow().len())
}

/// Test seam: buffers this thread allocated because its pool was enabled
/// but empty. Unchanged across a stretch of media means every PCM buffer in
/// it was reused.
#[doc(hidden)]
pub fn pcm_pool_misses_on_this_thread() -> u64 {
    THREAD_MISSES.with(Cell::get)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_rtp_core::executor::{
        override_media_executor_config_on_this_thread, MediaExecutorConfig,
    };

    fn drain_pool() {
        POOL.with(|pool| pool.borrow_mut().clear());
    }

    #[test]
    fn disabled_pool_allocates_like_the_code_it_replaces_and_keeps_nothing() {
        for config in [
            MediaExecutorConfig::v1(),
            MediaExecutorConfig::v2().with_pcm_pool(false),
        ] {
            let _config = override_media_executor_config_on_this_thread(config);
            drain_pool();
            assert_eq!(take_zeroed_pcm_buffer(160), vec![0i16; 160]);
            assert_eq!(take_pcm_copy(&[1, -2, 3]), vec![1i16, -2, 3]);
            recycle_pcm_buffer(vec![7; 160]);
            assert_eq!(pcm_pool_len_on_this_thread(), 0, "dropped, not pooled");
            let misses = pcm_pool_misses_on_this_thread();
            assert!(take_pcm_buffer(160).capacity() >= 160);
            assert_eq!(pcm_pool_misses_on_this_thread(), misses, "no miss counted");
        }
    }

    #[test]
    fn enabled_pool_reuses_buffers_and_hands_out_clean_contents() {
        let _v2 = override_media_executor_config_on_this_thread(MediaExecutorConfig::v2());
        drain_pool();
        recycle_pcm_buffer(vec![i16::MAX; 320]);
        assert_eq!(pcm_pool_len_on_this_thread(), 1);

        let zeroed = take_zeroed_pcm_buffer(160);
        assert_eq!(zeroed, vec![0i16; 160], "a dirty pooled buffer is zeroed");
        assert_eq!(pcm_pool_len_on_this_thread(), 0, "taken from the pool");
        recycle_pcm_buffer(zeroed);

        let copy = take_pcm_copy(&[5, 6, 7]);
        assert_eq!(copy, vec![5i16, 6, 7]);
        recycle_pcm_buffer(copy);

        let empty = take_pcm_buffer(500);
        assert!(empty.is_empty() && empty.capacity() >= 500);
        recycle_pcm_buffer(empty);
        assert_eq!(pcm_pool_len_on_this_thread(), 1);

        // Misses are counted only when the pool is empty.
        drain_pool();
        let misses = pcm_pool_misses_on_this_thread();
        let process_misses = pcm_pool_counters().misses;
        drop(take_pcm_buffer(1));
        assert_eq!(pcm_pool_misses_on_this_thread(), misses + 1);
        assert!(pcm_pool_counters().misses > process_misses);
        recycle_pcm_buffer(Vec::with_capacity(160));
        drop(take_pcm_buffer(160));
        assert_eq!(pcm_pool_misses_on_this_thread(), misses + 1);
    }

    #[test]
    fn the_pool_is_bounded_in_count_and_buffer_size() {
        let _v2 = override_media_executor_config_on_this_thread(MediaExecutorConfig::v2());
        drain_pool();
        recycle_pcm_buffer(Vec::new());
        recycle_pcm_buffer(Vec::with_capacity(PCM_POOL_MAX_SAMPLES + 1));
        assert_eq!(
            pcm_pool_len_on_this_thread(),
            0,
            "empty and oversized are freed"
        );
        for _ in 0..PCM_POOL_CAPACITY + 8 {
            recycle_pcm_buffer(Vec::with_capacity(160));
        }
        assert_eq!(pcm_pool_len_on_this_thread(), PCM_POOL_CAPACITY);
        drain_pool();
    }
}
