//! Recv buffer pool for the UDP RTP receive loop.
//!
//! Phase C23c: eliminates the per-packet `Bytes::copy_from_slice`
//! that `RtpPacket::parse` performed to take ownership of the
//! payload. Pre-allocated `Vec<u8>` recv buffers cycle through a
//! lock-free [`crossbeam_queue::ArrayQueue`]; each recv pulls a
//! buffer, fills it via `recv_from`, then hands it to
//! [`bytes::Bytes::from_owner`] so the downstream `RtpPacket::payload`
//! is a refcounted slice with zero allocations. When every clone of
//! that `Bytes` drops, the owner runs its `Drop` impl and returns
//! the underlying `Vec` to the pool.
//!
//! On the SRTP path the same buffer is handed in as `&[u8]` to
//! `SrtpContext::unprotect`, which allocates its own decrypted
//! `Bytes` — the pooled buf falls back to the pool immediately
//! after the unprotect returns, since no downstream consumer holds
//! a `Bytes` slice of it.
//!
//! Capacity sizing: defaults aim for "enough buffers in flight to
//! survive a brief downstream stall without forcing a fresh alloc".
//! Below the high-water mark the pool grows by allocating fresh
//! `Vec<u8>`s; above it, returned buffers are simply dropped.

use bytes::Bytes;
use crossbeam_queue::ArrayQueue;
use std::mem::ManuallyDrop;
#[cfg(feature = "memory-diagnostics")]
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Lock-free pool of recv buffers used by [`UdpRtpTransport`](super::UdpRtpTransport).
#[derive(Debug)]
pub struct RecvBufPool {
    pool: ArrayQueue<Vec<u8>>,
    buf_size: usize,
    #[cfg_attr(not(feature = "memory-diagnostics"), allow(dead_code))]
    max_in_flight: usize,
    #[cfg(feature = "memory-diagnostics")]
    allocated_total: AtomicU64,
    #[cfg(feature = "memory-diagnostics")]
    dropped_total: AtomicU64,
}

#[cfg(feature = "memory-diagnostics")]
#[derive(Debug, Clone, Copy, Default)]
pub struct RecvBufPoolDiagnosticCounts {
    pub idle_buffers: usize,
    pub idle_bytes: usize,
    pub buffer_size: usize,
    pub max_in_flight: usize,
    pub allocated_total: u64,
    pub dropped_total: u64,
}

impl RecvBufPool {
    /// Create a new recv pool with capacity for `max_in_flight`
    /// buffers, each `buf_size` bytes. The pool starts empty; the
    /// first `max_in_flight` `get()` calls allocate, subsequent
    /// calls reuse buffers returned via the `PooledRecvBuf` /
    /// `Bytes::from_owner` drop paths.
    pub fn new(max_in_flight: usize, buf_size: usize) -> Arc<Self> {
        Arc::new(Self {
            pool: ArrayQueue::new(max_in_flight.max(1)),
            buf_size,
            max_in_flight: max_in_flight.max(1),
            #[cfg(feature = "memory-diagnostics")]
            allocated_total: AtomicU64::new(0),
            #[cfg(feature = "memory-diagnostics")]
            dropped_total: AtomicU64::new(0),
        })
    }

    /// Pull a buffer from the pool, or allocate a fresh one if the
    /// pool is empty. Returns a RAII handle that auto-returns the
    /// buffer to the pool when dropped — unless ownership is
    /// explicitly transferred via [`PooledRecvBuf::into_bytes`].
    #[inline]
    pub fn get(self: &Arc<Self>) -> PooledRecvBuf {
        let buf = match self.pool.pop() {
            Some(mut buf) => {
                buf.resize(self.buf_size, 0);
                buf
            }
            None => {
                #[cfg(feature = "memory-diagnostics")]
                {
                    self.allocated_total.fetch_add(1, Ordering::Relaxed);
                    rvoip_infra_common::memory_diagnostics::record_created(
                        "rtp_core.recv_pool.buffer",
                        self.buf_size,
                    );
                }
                vec![0u8; self.buf_size]
            }
        };
        #[cfg(feature = "memory-diagnostics")]
        rvoip_infra_common::memory_diagnostics::record_checkout(
            "rtp_core.recv_pool.checkout",
            self.buf_size,
        );
        PooledRecvBuf {
            buf: Some(buf),
            pool: self.clone(),
        }
    }

    /// Number of buffers currently sitting in the pool (i.e. not in
    /// flight). Mostly useful for tests / diagnostics.
    pub fn idle_len(&self) -> usize {
        self.pool.len()
    }

    #[cfg(feature = "memory-diagnostics")]
    pub fn diagnostic_counts(&self) -> RecvBufPoolDiagnosticCounts {
        let idle = self.pool.len();
        RecvBufPoolDiagnosticCounts {
            idle_buffers: idle,
            idle_bytes: idle * self.buf_size,
            buffer_size: self.buf_size,
            max_in_flight: self.max_in_flight,
            allocated_total: self.allocated_total.load(Ordering::Relaxed),
            dropped_total: self.dropped_total.load(Ordering::Relaxed),
        }
    }

    /// Internal: actually push a buf back. Used by both the
    /// `PooledRecvBuf` and `Bytes::from_owner` drop paths.
    fn return_buf(&self, mut buf: Vec<u8>) {
        buf.resize(self.buf_size, 0);
        #[cfg(feature = "memory-diagnostics")]
        rvoip_infra_common::memory_diagnostics::record_return(
            "rtp_core.recv_pool.checkout",
            self.buf_size,
        );
        if self.pool.push(buf).is_err() {
            #[cfg(feature = "memory-diagnostics")]
            {
                self.dropped_total.fetch_add(1, Ordering::Relaxed);
                rvoip_infra_common::memory_diagnostics::record_dropped_full(
                    "rtp_core.recv_pool.buffer",
                    self.buf_size,
                );
                rvoip_infra_common::memory_diagnostics::record_dropped(
                    "rtp_core.recv_pool.buffer",
                    self.buf_size,
                );
            }
        }
    }
}

#[cfg(feature = "memory-diagnostics")]
impl Drop for RecvBufPool {
    fn drop(&mut self) {
        while self.pool.pop().is_some() {
            self.dropped_total.fetch_add(1, Ordering::Relaxed);
            rvoip_infra_common::memory_diagnostics::record_dropped(
                "rtp_core.recv_pool.buffer",
                self.buf_size,
            );
        }
    }
}

/// RAII handle for an in-flight recv buffer.
///
/// Dropping the handle returns the underlying `Vec<u8>` to the pool.
/// To hand ownership downstream as a refcounted `Bytes` view, use
/// [`Self::into_bytes`] — that wraps the buf in a
/// `Bytes::from_owner` whose Drop path also returns it to the pool.
pub struct PooledRecvBuf {
    buf: Option<Vec<u8>>,
    pool: Arc<RecvBufPool>,
}

impl PooledRecvBuf {
    /// Mutable byte slice for passing to `UdpSocket::recv_from`.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.buf
            .as_mut()
            .expect("PooledRecvBuf consumed")
            .as_mut_slice()
    }

    /// Immutable byte slice for callers that just need to inspect
    /// the recv'd bytes (e.g. `SrtpContext::unprotect`, which makes
    /// its own owned copy of the decrypted payload).
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        self.buf
            .as_ref()
            .expect("PooledRecvBuf consumed")
            .as_slice()
    }

    /// Transfer ownership into a `Bytes` view of `[0..size]`. When
    /// every clone of the returned `Bytes` drops, the underlying
    /// `Vec` returns to the pool. The handle is consumed; no Drop
    /// path runs after this point.
    pub fn into_bytes(mut self, size: usize) -> Bytes {
        let mut buf = self.buf.take().expect("PooledRecvBuf already consumed");
        debug_assert!(size <= buf.len(), "size {} > buf.len() {}", size, buf.len());
        buf.truncate(size);
        Bytes::from_owner(OwnedRecvBuf {
            buf: ManuallyDrop::new(buf),
            pool: self.pool.clone(),
        })
    }
}

impl Drop for PooledRecvBuf {
    fn drop(&mut self) {
        if let Some(buf) = self.buf.take() {
            self.pool.return_buf(buf);
        }
    }
}

/// Owner held by `Bytes::from_owner`. Returns its `Vec<u8>` to the
/// pool on Drop.
struct OwnedRecvBuf {
    buf: ManuallyDrop<Vec<u8>>,
    pool: Arc<RecvBufPool>,
}

impl AsRef<[u8]> for OwnedRecvBuf {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        &self.buf
    }
}

impl Drop for OwnedRecvBuf {
    fn drop(&mut self) {
        // SAFETY: `self.buf` is taken exactly once in Drop, never
        // accessed again after.
        let buf = unsafe { ManuallyDrop::take(&mut self.buf) };
        self.pool.return_buf(buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_handle_returns_buf_to_pool() {
        let pool = RecvBufPool::new(4, 1500);
        assert_eq!(pool.idle_len(), 0);
        {
            let _buf = pool.get();
            // _buf drops here, returning the buf.
        }
        assert_eq!(pool.idle_len(), 1);
    }

    #[test]
    fn into_bytes_returns_buf_after_bytes_drop() {
        let pool = RecvBufPool::new(4, 1500);
        let buf = pool.get();
        let bytes = buf.into_bytes(200);
        assert_eq!(bytes.len(), 200);
        // Bytes still alive — buf not yet back.
        assert_eq!(pool.idle_len(), 0);
        drop(bytes);
        // Now the owner dropped → buf back in pool.
        assert_eq!(pool.idle_len(), 1);
    }

    #[test]
    fn buf_is_reusable() {
        let pool = RecvBufPool::new(4, 1500);
        drop(pool.get());
        assert_eq!(pool.idle_len(), 1);
        let buf = pool.get();
        assert_eq!(buf.as_slice().len(), 1500);
        assert_eq!(pool.idle_len(), 0);
    }

    #[test]
    fn pool_caps_returned_buffers_at_capacity() {
        let pool = RecvBufPool::new(2, 1500);
        let a = pool.get();
        let b = pool.get();
        let c = pool.get();
        drop(a);
        drop(b);
        drop(c);
        // Two went back, the third was dropped (over capacity).
        assert_eq!(pool.idle_len(), 2);
    }
}
