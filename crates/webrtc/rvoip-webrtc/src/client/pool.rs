//! G3 — Signaling-connection pool keyed by base URL.
//!
//! The default [`WebRtcClient::call`](crate::client::WebRtcClient::call)
//! flow opens a fresh signaler per call. For applications that place many
//! concurrent calls to the same backend, opening one WebSocket per call is
//! wasteful. The pool returns a shared [`Signaler`](crate::client::Signaler)
//! per `ws_url`; entries idle for `idle_ttl` are evicted on the next
//! `get` request.
//!
//! Caller still owns the [`Signaler`] handle the pool returns — multiple
//! callers can hold the same `Arc<dyn Signaler>` concurrently (the
//! underlying [`WsSignaler`](crate::client::WsSignaler) muxes by
//! `connection_id`).
//!
//! ```ignore
//! let pool = SignalingPool::new(Duration::from_secs(60));
//! let sig_a = pool.get_ws("ws://server/signal").await?;
//! let sig_b = pool.get_ws("ws://server/signal").await?;
//! // sig_a and sig_b are the same Arc — one WebSocket, two callers.
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use parking_lot::Mutex;

use crate::errors::Result;

/// Lightweight signaling-connection pool. Cheap to clone (single Arc).
#[derive(Clone)]
pub struct SignalingPool {
    inner: Arc<Inner>,
}

struct Inner {
    /// Idle entries older than this are evicted lazily on the next `get`.
    idle_ttl: Duration,
    #[cfg(feature = "signaling-ws")]
    ws_entries: DashMap<String, Entry<crate::client::WsSignaler>>,
}

struct Entry<S> {
    signaler: Arc<S>,
    last_used: Mutex<Instant>,
}

impl<S> Entry<S> {
    fn new(s: Arc<S>) -> Self {
        Self {
            signaler: s,
            last_used: Mutex::new(Instant::now()),
        }
    }

    fn touch(&self) {
        *self.last_used.lock() = Instant::now();
    }

    fn idle_for(&self, now: Instant) -> Duration {
        now.saturating_duration_since(*self.last_used.lock())
    }
}

impl SignalingPool {
    /// Construct a pool with the given idle TTL. Set `Duration::ZERO` to
    /// keep entries forever (caller is responsible for explicit eviction).
    pub fn new(idle_ttl: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                idle_ttl,
                #[cfg(feature = "signaling-ws")]
                ws_entries: DashMap::new(),
            }),
        }
    }

    /// Reap entries idle for longer than `idle_ttl`.
    pub fn prune_idle(&self) {
        let ttl = self.inner.idle_ttl;
        if ttl.is_zero() {
            return;
        }
        let now = Instant::now();
        #[cfg(feature = "signaling-ws")]
        self.inner.ws_entries.retain(|_url, entry| entry.idle_for(now) < ttl);
    }

    /// Number of cached WS signaler entries (useful for tests + ops).
    pub fn ws_len(&self) -> usize {
        #[cfg(feature = "signaling-ws")]
        {
            self.inner.ws_entries.len()
        }
        #[cfg(not(feature = "signaling-ws"))]
        {
            0
        }
    }

    /// Acquire a shared [`WsSignaler`](crate::client::WsSignaler) for `ws_url`.
    /// Returns the cached entry if one exists, otherwise constructs a fresh
    /// signaler with default [`WsSignalerConfig`](crate::client::WsSignalerConfig).
    #[cfg(feature = "signaling-ws")]
    pub async fn get_ws(&self, ws_url: &str) -> Result<Arc<crate::client::WsSignaler>> {
        self.prune_idle();
        if let Some(entry) = self.inner.ws_entries.get(ws_url) {
            entry.touch();
            return Ok(Arc::clone(&entry.signaler));
        }
        let signaler = Arc::new(crate::client::WsSignaler::new(ws_url));
        let entry = Entry::new(Arc::clone(&signaler));
        self.inner.ws_entries.insert(ws_url.to_string(), entry);
        Ok(signaler)
    }

    /// Explicitly drop the cached entry for `ws_url`.
    pub fn evict(&self, ws_url: &str) {
        #[cfg(feature = "signaling-ws")]
        {
            self.inner.ws_entries.remove(ws_url);
        }
        #[cfg(not(feature = "signaling-ws"))]
        {
            let _ = ws_url;
        }
    }
}

#[cfg(all(test, feature = "signaling-ws"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pool_returns_cached_signaler_for_same_url() {
        let pool = SignalingPool::new(Duration::from_secs(60));
        let a = pool.get_ws("ws://127.0.0.1:1/sig").await.unwrap();
        let b = pool.get_ws("ws://127.0.0.1:1/sig").await.unwrap();
        assert!(
            Arc::ptr_eq(&a, &b),
            "second get for same URL must return the cached entry"
        );
        assert_eq!(pool.ws_len(), 1);
    }

    #[tokio::test]
    async fn different_urls_get_different_signalers() {
        let pool = SignalingPool::new(Duration::from_secs(60));
        let a = pool.get_ws("ws://127.0.0.1:1/sig").await.unwrap();
        let b = pool.get_ws("ws://127.0.0.1:2/sig").await.unwrap();
        assert!(!Arc::ptr_eq(&a, &b));
        assert_eq!(pool.ws_len(), 2);
    }

    #[tokio::test]
    async fn evict_drops_cached_entry() {
        let pool = SignalingPool::new(Duration::from_secs(60));
        let _ = pool.get_ws("ws://127.0.0.1:1/sig").await.unwrap();
        assert_eq!(pool.ws_len(), 1);
        pool.evict("ws://127.0.0.1:1/sig");
        assert_eq!(pool.ws_len(), 0);
    }
}
