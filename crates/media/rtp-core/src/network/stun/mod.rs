//! RFC 8489 STUN client (Binding Request / Response only).
//!
//! Hand-rolled minimal codec — no `webrtc-stun` / `stun_codec` dep.
//! The wire surface is small and the alternative (an external crate)
//! would obscure a load-bearing primitive that future ICE / TURN
//! work in this crate will share.
//!
//! ## Scope
//!
//! - `STUN Binding Request` encode (anonymous — no MESSAGE-INTEGRITY,
//!   no FINGERPRINT, no USERNAME). Sufficient for unauthenticated
//!   public STUN servers (Google, Cloudflare, etc.).
//! - `STUN Binding Response` decode for the IPv4 and IPv6
//!   `XOR-MAPPED-ADDRESS` attribute (RFC 8489 §14.2). Falls back to
//!   `MAPPED-ADDRESS` (§14.1) for legacy servers.
//! - One-shot async [`StunClient::discover`] that sends a request and
//!   waits for the matching response, with retry/timeout per RFC 8489
//!   §6.2.1 (RTO-driven, capped probe budget).
//!
//! ## Out of scope (deferred)
//!
//! - Long-term credential / MESSAGE-INTEGRITY / FINGERPRINT — Sprint
//!   4 D3 (ICE) prerequisite.
//! - Comprehension-required attribute handling on responses (RFC
//!   8489 §14, error 420). The response surface we accept is the
//!   common-case `XOR-MAPPED-ADDRESS` only.
//! - Server-side responder. We are a UAC.

mod message;

pub use message::{decode_binding_response, encode_binding_request, MAGIC_COOKIE};

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::time::timeout;

/// Errors surfaced by the STUN client.
#[derive(Debug, thiserror::Error)]
pub enum StunError {
    #[error("STUN message too short ({got} bytes; need at least 20)")]
    TooShort { got: usize },
    #[error("STUN magic cookie mismatch (got 0x{got:08x}; expected 0x{expected:08x})")]
    MagicCookieMismatch { got: u32, expected: u32 },
    #[error("STUN transaction id mismatch (request != response)")]
    TransactionIdMismatch,
    #[error("STUN message type 0x{0:04x} is not a Binding Response")]
    NotBindingResponse(u16),
    #[error("STUN response carried no MAPPED-ADDRESS or XOR-MAPPED-ADDRESS")]
    NoMappedAddress,
    #[error("STUN attribute family 0x{0:02x} unrecognised (expected 0x01 IPv4 or 0x02 IPv6)")]
    UnknownAddressFamily(u8),
    #[error("STUN attribute body too short ({got} bytes; need {need})")]
    AttributeTruncated { got: usize, need: usize },
    #[error("STUN probe timed out after {budget_ms} ms across {attempts} attempts")]
    ProbeTimeout { attempts: u32, budget_ms: u64 },
    #[error("STUN socket I/O error: {0}")]
    Io(#[from] io::Error),
}

/// Async STUN client. Borrows the same UDP socket the RTP path will
/// use, so the discovered NAT binding matches the binding the
/// outbound RTP packets actually traverse.
///
/// **Critical invariant:** the socket passed to [`Self::new`] MUST be
/// the same socket that subsequent RTP packets are sent from. A fresh
/// socket creates a fresh NAT binding and the discovered address
/// won't match the one carriers see on the audio packets.
pub struct StunClient {
    socket: Arc<UdpSocket>,
    server: SocketAddr,
    /// Per-attempt timeout. RFC 8489 §6.2.1 recommends RTO-based
    /// exponential backoff; we keep it constant here because the
    /// total budget caps the call regardless.
    attempt_timeout: Duration,
    /// Total wall-clock budget. After this elapses the probe gives up.
    total_budget: Duration,
}

impl StunClient {
    /// Construct a client. `socket` MUST be the bound RTP socket the
    /// caller will use for actual media traffic — see the type-level
    /// invariant.
    pub fn new(socket: Arc<UdpSocket>, server: SocketAddr) -> Self {
        Self {
            socket,
            server,
            attempt_timeout: Duration::from_millis(500),
            total_budget: Duration::from_millis(1_500),
        }
    }

    /// Override the per-attempt timeout (default 500 ms).
    pub fn with_attempt_timeout(mut self, t: Duration) -> Self {
        self.attempt_timeout = t;
        self
    }

    /// Override the total wall-clock budget (default 1500 ms).
    pub fn with_total_budget(mut self, t: Duration) -> Self {
        self.total_budget = t;
        self
    }

    /// Run a single Binding probe against the configured server,
    /// returning the public mapping the server saw. Retries within
    /// the configured budget; each attempt uses a fresh transaction
    /// id so a delayed late-arriving response from a prior attempt
    /// doesn't cause a false match.
    pub async fn discover(&self) -> Result<SocketAddr, StunError> {
        let started = std::time::Instant::now();
        let mut attempts: u32 = 0;
        let mut buf = [0u8; 1500];

        while started.elapsed() < self.total_budget {
            attempts += 1;

            let (request, txn_id) = encode_binding_request();
            self.socket.send_to(&request, self.server).await?;

            // Bound the wait by both the per-attempt timeout AND the
            // remaining total budget — whichever is tighter wins.
            let remaining = self.total_budget.saturating_sub(started.elapsed());
            let wait = self.attempt_timeout.min(remaining);
            if wait.is_zero() {
                break;
            }

            match timeout(wait, self.socket.recv_from(&mut buf)).await {
                Ok(Ok((n, peer))) => {
                    if peer != self.server {
                        // Stray packet from somewhere else (e.g. an
                        // RTP peer racing the STUN probe). Ignore and
                        // keep waiting until our budget elapses.
                        tracing::trace!(
                            "STUN: ignoring packet from non-server {} (expected {})",
                            peer,
                            self.server
                        );
                        continue;
                    }
                    match decode_binding_response(&buf[..n], &txn_id) {
                        Ok(addr) => return Ok(addr),
                        Err(StunError::TransactionIdMismatch) => {
                            // Late response from a prior attempt; ignore.
                            tracing::trace!("STUN: ignored stale response (txn id mismatch)");
                            continue;
                        }
                        Err(e) => return Err(e),
                    }
                }
                Ok(Err(e)) => return Err(StunError::Io(e)),
                Err(_) => {
                    // Per-attempt timeout — loop and try again.
                    tracing::debug!(
                        "STUN: attempt {} timed out after {:?}, retrying",
                        attempts,
                        wait
                    );
                }
            }
        }

        Err(StunError::ProbeTimeout {
            attempts,
            budget_ms: self.total_budget.as_millis() as u64,
        })
    }
}
