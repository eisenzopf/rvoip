//! Envelope-id round-trip correlation.
//!
//! Senders that need to await a response register an `EnvelopeId` here
//! before sending; the receiver dispatches incoming envelopes by their
//! `in_reply_to` field. Per design doc §3.7 the default TTL is 30s,
//! matching CONVERSATION_PROTOCOL.md §7.3's reconnect grace window.

use std::time::Duration;

use dashmap::DashMap;
use tokio::sync::oneshot;
use tokio::time::timeout;

use crate::envelope::UctpEnvelope;
use crate::errors::SubstrateError;
use crate::ids::EnvelopeId;
use crate::types::MessageType;

struct PendingEntry {
    sender: oneshot::Sender<UctpEnvelope>,
    expectation: Option<ReplyExpectation>,
}

#[derive(Clone)]
struct ReplyExpectation {
    sid: Option<String>,
    connid: Option<String>,
    request_type: MessageType,
}

impl ReplyExpectation {
    fn for_request(request: &UctpEnvelope) -> Self {
        Self {
            sid: request.sid.clone(),
            connid: request.connid.clone(),
            request_type: request.msg_type.clone(),
        }
    }

    fn matches(&self, reply: &UctpEnvelope) -> bool {
        self.sid == reply.sid
            && self.connid == reply.connid
            && (matches!(reply.msg_type, MessageType::Ack | MessageType::Error)
                || reply.msg_type == self.request_type)
    }
}

#[derive(Default)]
pub struct Pending {
    inner: DashMap<EnvelopeId, PendingEntry>,
}

impl Pending {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register interest in a response to `id` and await it (up to `ttl`).
    pub async fn wait_for(
        &self,
        id: EnvelopeId,
        ttl: Duration,
    ) -> Result<UctpEnvelope, SubstrateError> {
        let (tx, rx) = oneshot::channel();
        self.inner.insert(
            id.clone(),
            PendingEntry {
                sender: tx,
                expectation: None,
            },
        );
        match timeout(ttl, rx).await {
            Ok(Ok(env)) => Ok(env),
            Ok(Err(_)) => {
                // Sender dropped — surfaces as Closed.
                self.inner.remove(&id);
                Err(SubstrateError::Closed)
            }
            Err(_) => {
                self.inner.remove(&id);
                Err(SubstrateError::Closed)
            }
        }
    }

    /// Match an inbound envelope's `in_reply_to` against a pending entry.
    /// Returns `Ok(())` when delivered, or `Err(env)` to give the
    /// envelope back to the caller for normal inbound routing.
    pub fn deliver(&self, env: UctpEnvelope) -> Result<(), UctpEnvelope> {
        let Some(reply_to) = env.in_reply_to.as_ref() else {
            return Err(env);
        };
        let key = EnvelopeId::from_string(reply_to.clone());
        if self.inner.get(&key).is_some_and(|entry| {
            entry
                .expectation
                .as_ref()
                .is_some_and(|expectation| !expectation.matches(&env))
        }) {
            return Err(env);
        }
        match self.inner.remove(&key) {
            Some((_, entry)) => {
                // If the receiver is gone, the response is dropped.
                let _ = entry.sender.send(env);
                Ok(())
            }
            None => Err(env),
        }
    }

    /// Whether `in_reply_to` currently names a locally registered request.
    /// Coordinators use this read-only check to apply authentication,
    /// signature, replay, and scope gates before removing the waiter.
    pub fn has_waiter(&self, in_reply_to: &str) -> bool {
        self.inner
            .contains_key(&EnvelopeId::from_string(in_reply_to.to_owned()))
    }

    /// Drop every pending waiter; used during coordinator shutdown.
    pub fn close(&self) {
        self.inner.clear();
    }

    /// Number of outstanding correlated requests. Surfaced as the
    /// `uctp_substrate_pending_outstanding` gauge per design doc §3.9
    /// so leak detection on request/response flows (renegotiate-media,
    /// future DPoP step-up) has a real-time signal.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// `true` when no correlated requests are outstanding.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

/// Gap plan §4.2 v1 punch list — convenience helper for adapter
/// methods that need to send an envelope and synchronously await a
/// correlated reply (`in_reply_to == request.id`). Used by the
/// QUIC/WT/WS adapters' `renegotiate_media` impls.
///
/// Steps: register on `pending` first (so a fast reply doesn't race
/// us), send the envelope on `out_tx`, await the reply up to `ttl`.
/// On send error or timeout the registration is rolled back by
/// `Pending::wait_for`'s own cleanup path.
pub async fn send_and_wait(
    out_tx: &tokio::sync::mpsc::Sender<UctpEnvelope>,
    pending: &Pending,
    env: UctpEnvelope,
    ttl: Duration,
) -> Result<UctpEnvelope, SubstrateError> {
    let req_id = crate::ids::EnvelopeId::from_string(env.id.clone());
    let expectation = ReplyExpectation::for_request(&env);
    // Register before sending so an immediate reply doesn't fire
    // through deliver() into an empty map.
    let (tx, rx) = tokio::sync::oneshot::channel();
    pending.inner.insert(
        req_id.clone(),
        PendingEntry {
            sender: tx,
            expectation: Some(expectation),
        },
    );
    if out_tx.send(env).await.is_err() {
        pending.inner.remove(&req_id);
        return Err(SubstrateError::Closed);
    }
    match tokio::time::timeout(ttl, rx).await {
        Ok(Ok(reply)) => Ok(reply),
        Ok(Err(_)) => {
            pending.inner.remove(&req_id);
            Err(SubstrateError::Closed)
        }
        Err(_) => {
            pending.inner.remove(&req_id);
            Err(SubstrateError::Closed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::MessageType;
    use chrono::Utc;

    fn env_with(id: &str, in_reply_to: Option<&str>) -> UctpEnvelope {
        UctpEnvelope {
            v: 1,
            msg_type: MessageType::Ack,
            id: id.into(),
            ts: Utc::now(),
            cid: None,
            sid: None,
            connid: None,
            in_reply_to: in_reply_to.map(String::from),
            payload: serde_json::Value::Null,
            signature: None,
        }
    }

    #[tokio::test]
    async fn wait_for_returns_on_deliver() {
        let p = std::sync::Arc::new(Pending::new());
        let req_id = EnvelopeId::new();
        let id_for_task = req_id.clone();
        let p_in = p.clone();
        let task =
            tokio::spawn(async move { p_in.wait_for(id_for_task, Duration::from_secs(5)).await });

        // Tiny yield so the wait task registers before deliver.
        tokio::time::sleep(Duration::from_millis(10)).await;

        let env = env_with("env_reply", Some(req_id.as_str()));
        assert!(p.deliver(env).is_ok());

        let got = task.await.unwrap().unwrap();
        assert_eq!(got.id, "env_reply");
    }

    #[tokio::test]
    async fn wait_for_times_out() {
        let p = Pending::new();
        let result = p
            .wait_for(EnvelopeId::new(), Duration::from_millis(50))
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn deliver_returns_env_when_no_pending_entry() {
        let p = Pending::new();
        let env = env_with("env_x", Some("env_y"));
        let returned = p.deliver(env).unwrap_err();
        assert_eq!(returned.in_reply_to.as_deref(), Some("env_y"));
    }

    #[tokio::test]
    async fn send_and_wait_rejects_reply_for_another_resource_or_type() {
        let pending = std::sync::Arc::new(Pending::new());
        let (out_tx, mut out_rx) = tokio::sync::mpsc::channel(1);
        let request = UctpEnvelope::new(MessageType::ConnectionUpdate, serde_json::Value::Null)
            .with_sid("sid-a")
            .with_connid("conn-a");
        let request_id = request.id.clone();
        let pending_for_wait = std::sync::Arc::clone(&pending);
        let waiter = tokio::spawn(async move {
            send_and_wait(
                &out_tx,
                pending_for_wait.as_ref(),
                request,
                Duration::from_secs(1),
            )
            .await
        });
        let _sent = out_rx.recv().await.expect("request should be sent");

        let wrong_sid = UctpEnvelope::new(MessageType::ConnectionUpdate, serde_json::Value::Null)
            .with_sid("sid-b")
            .with_connid("conn-a")
            .with_in_reply_to(request_id.clone());
        assert!(pending.deliver(wrong_sid).is_err());
        assert_eq!(pending.len(), 1, "mismatched reply must retain waiter");

        let wrong_type = UctpEnvelope::new(MessageType::DtmfSend, serde_json::Value::Null)
            .with_sid("sid-a")
            .with_connid("conn-a")
            .with_in_reply_to(request_id.clone());
        assert!(pending.deliver(wrong_type).is_err());
        assert_eq!(pending.len(), 1, "wrong reply type must retain waiter");

        let valid = UctpEnvelope::new(MessageType::Ack, serde_json::Value::Null)
            .with_sid("sid-a")
            .with_connid("conn-a")
            .with_in_reply_to(request_id);
        assert!(pending.deliver(valid).is_ok());
        assert_eq!(waiter.await.unwrap().unwrap().msg_type, MessageType::Ack);
    }
}
