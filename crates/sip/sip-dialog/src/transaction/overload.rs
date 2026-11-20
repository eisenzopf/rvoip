//! Early, stateless admission for new dialog-forming INVITEs.
//!
//! An overloaded server must refuse excess new calls far more cheaply than it
//! admits them, or shedding itself becomes the overload. [`NewCallAdmission`]
//! classifies requests cheaply, lets in-dialog requests, retransmissions, ACK
//! and CANCEL through, and answers excess *new* INVITEs with a stateless
//! `503 Service Unavailable` (RFC 3261 §8.2.6) before any transaction,
//! dialog, session or media state exists.
//!
//! Each [`TransactionManager`](crate::transaction::TransactionManager) owns
//! one gate. The transaction layer only reads it; the application, which can
//! see sessions, memory and executor health, sets the [`AdmissionMode`] and
//! [`AdmissionLevel`] at run time. Fractional shedding is deterministic per
//! Call-ID while the level is unchanged, so a retransmitted INVITE gets an
//! identical 503 (same To tag), and the ACK for that 503 is absorbed without
//! reaching the transaction user.
//!
//! Over UDP, TCP and WebSocket the gate runs on the transaction manager's
//! dispatch path, before the request can create a server transaction. Over
//! TLS it can run earlier still, on the connection reader before the request
//! enters the bounded transport event queue: see
//! [`NewCallAdmission::tls_ingress_hook`].

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::retry_after::RetryAfter;
use rvoip_sip_transport::{IngressAdmissionHook, IngressVerdict, Transport, TransportRoute};
use sha2::{Digest, Sha256};

/// To-tag prefix of every stateless overload 503 sent by this crate. An ACK
/// carrying it acknowledges such a 503 and is absorbed.
pub const OVERLOAD_TO_TAG_PREFIX: &str = "rvoip-ovl-";
const OVERLOAD_TO_TAG_DOMAIN: &[u8] = b"rvoip-overload-503-v1";
/// A rejection must never stall the dispatcher behind a busy shared writer.
/// Past this budget the 503 is dropped and counted; the peer's own
/// retransmission gets another chance.
const OVERLOAD_REJECT_SEND_BUDGET: Duration = Duration::from_millis(25);

/// How a [`NewCallAdmission`] gate applies its decisions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum AdmissionMode {
    /// Every new INVITE proceeds. This is the default.
    #[default]
    Off,
    /// Decisions are computed and counted as
    /// [`would_reject`](NewCallAdmissionStats::would_reject), never applied.
    Observe,
    /// Rejections are answered with a stateless 503.
    Enforce,
}

impl AdmissionMode {
    fn as_u8(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::Observe => 1,
            Self::Enforce => 2,
        }
    }

    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Observe,
            2 => Self::Enforce,
            _ => Self::Off,
        }
    }
}

/// Ceiling on admitted new calls per second, with a burst allowance.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct NewCallRateLimit {
    /// Sustained admitted new calls per second. Must be finite and positive
    /// for the limit to apply.
    pub max_cps: f64,
    /// New calls admitted back to back before the sustained rate applies.
    /// Values below 1 are treated as 1.
    pub burst: f64,
}

impl NewCallRateLimit {
    /// A limit of `max_cps` sustained new calls per second with a burst of
    /// `burst` calls.
    pub fn new(max_cps: f64, burst: f64) -> Self {
        Self { max_cps, burst }
    }
}

/// Initial configuration of a [`NewCallAdmission`] gate.
///
/// The default is [`AdmissionMode::Off`] with no rate limit: every new INVITE
/// reaches the transaction user, as before this gate existed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct NewCallAdmissionConfig {
    /// Initial mode. Default: [`AdmissionMode::Off`].
    pub mode: AdmissionMode,
    /// Initial rate limit. Default: `None`.
    pub rate_limit: Option<NewCallRateLimit>,
}

impl NewCallAdmissionConfig {
    /// A configuration with `mode` and no rate limit.
    pub fn new(mode: AdmissionMode) -> Self {
        Self {
            mode,
            rate_limit: None,
        }
    }

    /// Cap admitted new calls with `limit`.
    pub fn with_rate_limit(mut self, limit: NewCallRateLimit) -> Self {
        self.rate_limit = Some(limit);
        self
    }

    /// True unless the mode is [`AdmissionMode::Off`].
    pub fn is_enabled(&self) -> bool {
        self.mode != AdmissionMode::Off
    }
}

/// The level an application controller publishes to a gate.
///
/// A closed gate rejects every new INVITE. An open gate rejects
/// `shed_per_million` of Call-IDs, chosen by a stable hash so retransmissions
/// get the same answer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct AdmissionLevel {
    /// Reject every new INVITE.
    pub closed: bool,
    /// Fraction of new Call-IDs to reject, in parts per million (capped at
    /// 1,000,000). Ignored while closed.
    pub shed_per_million: u32,
    /// `Retry-After` seconds on rejections made at this level; `None` omits
    /// the header.
    pub retry_after_secs: Option<u32>,
}

impl AdmissionLevel {
    /// Admit every new INVITE (subject to the rate limit).
    pub fn open() -> Self {
        Self::default()
    }

    /// Reject every new INVITE.
    pub fn closed(retry_after_secs: Option<u32>) -> Self {
        Self {
            closed: true,
            shed_per_million: 0,
            retry_after_secs,
        }
    }

    /// Reject `shed_per_million` of new Call-IDs.
    pub fn shedding(shed_per_million: u32, retry_after_secs: Option<u32>) -> Self {
        Self {
            closed: false,
            shed_per_million: shed_per_million.min(1_000_000),
            retry_after_secs,
        }
    }
}

/// Outcome of [`NewCallAdmission::decide`] for one new INVITE.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AdmissionDecision {
    /// Let the INVITE create its transaction.
    Admit,
    /// Answer with a stateless 503; `retry_after_secs` of `None` omits the
    /// `Retry-After` header.
    Reject {
        /// `Retry-After` seconds for the 503.
        retry_after_secs: Option<u32>,
    },
}

/// Snapshot of a gate's configuration and counters.
///
/// The counters are read one at a time, not as one atomic snapshot, so
/// consumers should work with deltas and tolerate a few calls of skew.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct NewCallAdmissionStats {
    /// Current mode.
    pub mode: AdmissionMode,
    /// Current level.
    pub level: AdmissionLevel,
    /// Current rate limit.
    pub rate_limit: Option<NewCallRateLimit>,
    /// New INVITEs decided while the mode was not [`AdmissionMode::Off`].
    pub decisions: u64,
    /// New INVITEs rejected under [`AdmissionMode::Enforce`].
    pub rejected: u64,
    /// New INVITEs that would have been rejected under
    /// [`AdmissionMode::Observe`].
    pub would_reject: u64,
    /// Rejections, or would-be rejections, caused by the rate limit.
    pub rate_limited: u64,
    /// Rejections answered directly by a TLS connection reader (a subset of
    /// `rejected`).
    pub tls_ingress_rejected: u64,
    /// ACKs for stateless overload 503s absorbed before the transaction user.
    pub acks_absorbed: u64,
    /// Stateless 503s that could not be sent within their budget.
    pub reject_sends_dropped: u64,
    /// True once [`NewCallAdmission::tls_ingress_hook`] was attached to a TLS
    /// transport.
    pub tls_ingress_attached: bool,
}

impl NewCallAdmissionStats {
    /// New INVITEs let through (`decisions - rejected`). Under
    /// [`AdmissionMode::Observe`] this includes would-be rejections.
    pub fn admitted(&self) -> u64 {
        self.decisions.saturating_sub(self.rejected)
    }
}

/// A transaction manager's new-call admission gate.
///
/// All state is atomics, so deciding is lock-free and an application may
/// change the mode, level and rate limit at any time from any thread.
#[derive(Debug)]
pub struct NewCallAdmission {
    mode: AtomicU8,
    closed: AtomicBool,
    shed_per_million: AtomicU32,
    /// `Retry-After` for rejections at the current level (0 omits it).
    retry_after_secs: AtomicU32,
    /// GCRA emission interval in nanoseconds (0 disables the rate limit).
    rate_interval_ns: AtomicU64,
    /// GCRA burst tolerance in nanoseconds.
    rate_tolerance_ns: AtomicU64,
    /// GCRA theoretical arrival time in nanoseconds since `epoch`.
    rate_tat_ns: AtomicU64,
    rate_max_cps_bits: AtomicU64,
    rate_burst_bits: AtomicU64,
    epoch: Instant,
    decisions: AtomicU64,
    rejected: AtomicU64,
    would_reject: AtomicU64,
    rate_limited: AtomicU64,
    tls_ingress_rejected: AtomicU64,
    acks_absorbed: AtomicU64,
    reject_sends_dropped: AtomicU64,
    tls_ingress_attached: AtomicBool,
}

impl Default for NewCallAdmission {
    fn default() -> Self {
        Self::new(NewCallAdmissionConfig::default())
    }
}

impl NewCallAdmission {
    /// A gate with `config`'s mode and rate limit and an open level.
    pub fn new(config: NewCallAdmissionConfig) -> Self {
        let gate = Self {
            mode: AtomicU8::new(config.mode.as_u8()),
            closed: AtomicBool::new(false),
            shed_per_million: AtomicU32::new(0),
            retry_after_secs: AtomicU32::new(0),
            rate_interval_ns: AtomicU64::new(0),
            rate_tolerance_ns: AtomicU64::new(0),
            rate_tat_ns: AtomicU64::new(0),
            rate_max_cps_bits: AtomicU64::new(0),
            rate_burst_bits: AtomicU64::new(0),
            epoch: Instant::now(),
            decisions: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            would_reject: AtomicU64::new(0),
            rate_limited: AtomicU64::new(0),
            tls_ingress_rejected: AtomicU64::new(0),
            acks_absorbed: AtomicU64::new(0),
            reject_sends_dropped: AtomicU64::new(0),
            tls_ingress_attached: AtomicBool::new(false),
        };
        gate.set_rate_limit(config.rate_limit);
        gate
    }

    /// Current mode.
    pub fn mode(&self) -> AdmissionMode {
        AdmissionMode::from_u8(self.mode.load(Ordering::Relaxed))
    }

    /// Change the mode.
    pub fn set_mode(&self, mode: AdmissionMode) {
        self.mode.store(mode.as_u8(), Ordering::Relaxed);
    }

    /// Current level.
    pub fn level(&self) -> AdmissionLevel {
        AdmissionLevel {
            closed: self.closed.load(Ordering::Acquire),
            shed_per_million: self.shed_per_million.load(Ordering::Relaxed),
            retry_after_secs: nonzero(self.retry_after_secs.load(Ordering::Relaxed)),
        }
    }

    /// Publish a new level.
    pub fn set_level(&self, level: AdmissionLevel) {
        self.shed_per_million
            .store(level.shed_per_million.min(1_000_000), Ordering::Relaxed);
        self.retry_after_secs
            .store(level.retry_after_secs.unwrap_or(0), Ordering::Relaxed);
        self.closed.store(level.closed, Ordering::Release);
    }

    /// Cap admitted new calls, or remove the cap with `None`. A limit whose
    /// `max_cps` is not finite and positive also removes the cap.
    pub fn set_rate_limit(&self, limit: Option<NewCallRateLimit>) {
        let Some(limit) = limit.filter(|limit| limit.max_cps.is_finite() && limit.max_cps > 0.0)
        else {
            self.rate_interval_ns.store(0, Ordering::Relaxed);
            return;
        };
        let burst = if limit.burst.is_finite() {
            limit.burst.max(1.0)
        } else {
            1.0
        };
        let interval = (1e9 / limit.max_cps).max(1.0) as u64;
        let tolerance = (interval as f64 * burst) as u64;
        self.rate_max_cps_bits
            .store(limit.max_cps.to_bits(), Ordering::Relaxed);
        self.rate_burst_bits
            .store(burst.to_bits(), Ordering::Relaxed);
        self.rate_tolerance_ns.store(tolerance, Ordering::Relaxed);
        self.rate_interval_ns.store(interval, Ordering::Release);
    }

    fn rate_limit(&self) -> Option<NewCallRateLimit> {
        (self.rate_interval_ns.load(Ordering::Acquire) != 0).then(|| {
            NewCallRateLimit::new(
                f64::from_bits(self.rate_max_cps_bits.load(Ordering::Relaxed)),
                f64::from_bits(self.rate_burst_bits.load(Ordering::Relaxed)),
            )
        })
    }

    fn clock_ns(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    /// Lock-free GCRA: true when one more call fits under the rate limit.
    fn rate_allows(&self) -> bool {
        let interval = self.rate_interval_ns.load(Ordering::Acquire);
        if interval == 0 {
            return true;
        }
        let tolerance = self.rate_tolerance_ns.load(Ordering::Relaxed);
        let now = self.clock_ns();
        let mut tat = self.rate_tat_ns.load(Ordering::Relaxed);
        loop {
            let next = tat.max(now).saturating_add(interval);
            if next.saturating_sub(now) > tolerance.saturating_add(interval) {
                return false;
            }
            match self.rate_tat_ns.compare_exchange_weak(
                tat,
                next,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(current) => tat = current,
            }
        }
    }

    /// Decide for one new dialog-forming INVITE identified by `call_id`.
    ///
    /// Under [`AdmissionMode::Observe`] a rejection is counted and
    /// [`AdmissionDecision::Admit`] is returned.
    pub fn decide(&self, call_id: &str) -> AdmissionDecision {
        let mode = self.mode();
        if mode == AdmissionMode::Off {
            return AdmissionDecision::Admit;
        }
        self.decisions.fetch_add(1, Ordering::Relaxed);
        let decision = if self.closed.load(Ordering::Acquire) {
            AdmissionDecision::Reject {
                retry_after_secs: nonzero(self.retry_after_secs.load(Ordering::Relaxed)),
            }
        } else {
            let shed = self.shed_per_million.load(Ordering::Relaxed);
            if shed > 0 && call_id_bucket(call_id) < shed {
                AdmissionDecision::Reject {
                    retry_after_secs: nonzero(self.retry_after_secs.load(Ordering::Relaxed)),
                }
            } else if !self.rate_allows() {
                self.rate_limited.fetch_add(1, Ordering::Relaxed);
                AdmissionDecision::Reject {
                    retry_after_secs: None,
                }
            } else {
                AdmissionDecision::Admit
            }
        };
        match (mode, decision) {
            (_, AdmissionDecision::Admit) => AdmissionDecision::Admit,
            (AdmissionMode::Observe, AdmissionDecision::Reject { .. }) => {
                self.would_reject.fetch_add(1, Ordering::Relaxed);
                AdmissionDecision::Admit
            }
            (_, reject) => {
                self.rejected.fetch_add(1, Ordering::Relaxed);
                reject
            }
        }
    }

    /// Snapshot of the configuration and counters.
    pub fn stats(&self) -> NewCallAdmissionStats {
        NewCallAdmissionStats {
            mode: self.mode(),
            level: self.level(),
            rate_limit: self.rate_limit(),
            decisions: self.decisions.load(Ordering::Relaxed),
            rejected: self.rejected.load(Ordering::Relaxed),
            would_reject: self.would_reject.load(Ordering::Relaxed),
            rate_limited: self.rate_limited.load(Ordering::Relaxed),
            tls_ingress_rejected: self.tls_ingress_rejected.load(Ordering::Relaxed),
            acks_absorbed: self.acks_absorbed.load(Ordering::Relaxed),
            reject_sends_dropped: self.reject_sends_dropped.load(Ordering::Relaxed),
            tls_ingress_attached: self.tls_ingress_attached(),
        }
    }

    /// A TLS ingress hook that applies this gate on the connection reader.
    ///
    /// Install it with
    /// [`TlsFlowConfig::with_ingress_admission`](rvoip_sip_transport::TlsFlowConfig::with_ingress_admission).
    /// New INVITEs are then decided, and rejected statelessly, before they
    /// enter the bounded transport event queue, and ACKs for those 503s are
    /// absorbed there. TLS requests that pass are not decided a second time
    /// by the transaction layer.
    ///
    /// [`TransportManager`](crate::transaction::transport::TransportManager)
    /// attaches this hook itself when its
    /// [`new_call_admission`](crate::transaction::transport::TransportManagerConfig::new_call_admission)
    /// mode is not [`AdmissionMode::Off`] and the application did not supply
    /// its own hook.
    pub fn tls_ingress_hook(self: &Arc<Self>) -> IngressAdmissionHook {
        self.tls_ingress_attached.store(true, Ordering::Release);
        let gate = Arc::clone(self);
        IngressAdmissionHook::new(move |message, _source| gate.ingress_verdict(message))
    }

    pub(crate) fn tls_ingress_attached(&self) -> bool {
        self.tls_ingress_attached.load(Ordering::Acquire)
    }

    pub(crate) fn ingress_verdict(&self, message: &Message) -> IngressVerdict {
        let Message::Request(request) = message else {
            return IngressVerdict::Pass;
        };
        if is_overload_reject_ack(request) {
            self.record_ack_absorbed();
            return IngressVerdict::Absorb;
        }
        if !is_new_dialog_invite(request) {
            return IngressVerdict::Pass;
        }
        match self.decide(call_id_of(request)) {
            AdmissionDecision::Admit => IngressVerdict::Pass,
            AdmissionDecision::Reject { retry_after_secs } => {
                self.tls_ingress_rejected.fetch_add(1, Ordering::Relaxed);
                let response = build_overload_response(request, retry_after_secs);
                IngressVerdict::Respond(Bytes::from(Message::Response(response).to_bytes()))
            }
        }
    }

    pub(crate) fn record_ack_absorbed(&self) {
        self.acks_absorbed.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_reject_send_dropped(&self) {
        self.reject_sends_dropped.fetch_add(1, Ordering::Relaxed);
    }
}

fn nonzero(value: u32) -> Option<u32> {
    (value > 0).then_some(value)
}

pub(crate) fn call_id_of(request: &Request) -> &str {
    request.call_id().map(|id| id.as_str()).unwrap_or("")
}

/// Stable 0..1,000,000 bucket for a Call-ID (FNV-1a; not security relevant).
fn call_id_bucket(call_id: &str) -> u32 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in call_id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    (hash % 1_000_000) as u32
}

/// True when `request` is a new dialog-forming INVITE (no To tag).
pub fn is_new_dialog_invite(request: &Request) -> bool {
    request.method() == Method::Invite && request.to().is_some_and(|to| to.tag().is_none())
}

/// True when `request` is an ACK for a stateless overload 503 from this
/// crate.
pub fn is_overload_reject_ack(request: &Request) -> bool {
    request.method() == Method::Ack
        && request
            .to()
            .and_then(|to| to.tag())
            .is_some_and(|tag| tag.starts_with(OVERLOAD_TO_TAG_PREFIX))
}

/// Deterministic local To tag for a rejection of `request`, so every
/// retransmission of one INVITE is answered identically.
fn overload_to_tag(request: &Request) -> String {
    let mut hasher = Sha256::new();
    let mut component = |bytes: &[u8]| {
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    };
    component(OVERLOAD_TO_TAG_DOMAIN);
    let branch = request
        .first_via()
        .and_then(|via| via.branch().map(str::to_owned));
    component(branch.as_deref().unwrap_or("").as_bytes());
    component(call_id_of(request).as_bytes());
    component(
        request
            .from()
            .and_then(|from| from.tag())
            .unwrap_or("")
            .as_bytes(),
    );
    component(
        &request
            .cseq()
            .map(|cseq| cseq.sequence())
            .unwrap_or(0)
            .to_be_bytes()[..],
    );
    let digest = hasher.finalize();
    let mut tag = String::with_capacity(OVERLOAD_TO_TAG_PREFIX.len() + 32);
    tag.push_str(OVERLOAD_TO_TAG_PREFIX);
    for byte in &digest[..16] {
        tag.push_str(&format!("{byte:02x}"));
    }
    tag
}

/// Build the RFC 3261 §8.2.6 stateless 503 for `request`: every Via in order,
/// From, To with a deterministic local tag, Call-ID, CSeq, optional
/// `Retry-After` and `Content-Length: 0`. No Record-Route or Contact: the
/// response does not form a dialog.
pub fn build_overload_response(request: &Request, retry_after_secs: Option<u32>) -> Response {
    let mut builder = ResponseBuilder::new(StatusCode::ServiceUnavailable, None);
    for header in request
        .headers
        .iter()
        .filter(|header| matches!(header, TypedHeader::Via(_)))
    {
        builder = builder.header(header.clone());
    }
    if let Some(header) = request.header(&HeaderName::From) {
        builder = builder.header(header.clone());
    }
    if let Some(to) = request.to() {
        let to = if to.tag().is_none() {
            to.clone().with_tag(overload_to_tag(request))
        } else {
            to.clone()
        };
        builder = builder.header(TypedHeader::To(to));
    }
    if let Some(header) = request.header(&HeaderName::CallId) {
        builder = builder.header(header.clone());
    }
    if let Some(header) = request.header(&HeaderName::CSeq) {
        builder = builder.header(header.clone());
    }
    if let Some(seconds) = retry_after_secs {
        builder = builder.header(TypedHeader::RetryAfter(RetryAfter::new(seconds)));
    }
    builder = builder.header(TypedHeader::ContentLength(ContentLength::new(0)));
    builder.build()
}

/// Send a stateless overload 503 within a fixed budget. Never fails the
/// caller; returns `false` when the 503 could not be sent, so the caller can
/// count it. The peer will retransmit.
pub(crate) async fn send_overload_reject(
    request: &Request,
    route: TransportRoute,
    transport: &Arc<dyn Transport>,
    retry_after_secs: Option<u32>,
) -> bool {
    let response = build_overload_response(request, retry_after_secs);
    let send = transport.send_message_via(Message::Response(response), route);
    matches!(
        tokio::time::timeout(OVERLOAD_REJECT_SEND_BUDGET, send).await,
        Ok(Ok(()))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_request(raw: &str) -> Request {
        match rvoip_sip_core::parse_message(raw.as_bytes()).expect("parse request") {
            Message::Request(request) => request,
            Message::Response(_) => panic!("expected a request"),
        }
    }

    fn invite(to_tag: Option<&str>) -> Request {
        let to = match to_tag {
            Some(tag) => format!("To: <sip:bob@example.com>;tag={tag}\r\n"),
            None => "To: <sip:bob@example.com>\r\n".to_string(),
        };
        parse_request(&format!(
            "INVITE sip:bob@example.com SIP/2.0\r\n\
             Via: SIP/2.0/TLS proxy.example.com:5061;branch=z9hG4bKproxy1\r\n\
             Via: SIP/2.0/TLS 10.0.0.1:5061;branch=z9hG4bKorigin1\r\n\
             Max-Forwards: 69\r\n\
             From: <sip:alice@example.com>;tag=fromtag1\r\n\
             {to}\
             Call-ID: call-1@example.com\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Length: 0\r\n\r\n"
        ))
    }

    fn ack_for(response: &Response) -> Request {
        let tag = response.to().and_then(|to| to.tag()).unwrap().to_owned();
        parse_request(&format!(
            "ACK sip:bob@example.com SIP/2.0\r\n\
             Via: SIP/2.0/TLS proxy.example.com:5061;branch=z9hG4bKproxy1\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:alice@example.com>;tag=fromtag1\r\n\
             To: <sip:bob@example.com>;tag={tag}\r\n\
             Call-ID: call-1@example.com\r\n\
             CSeq: 1 ACK\r\n\
             Content-Length: 0\r\n\r\n"
        ))
    }

    fn gate(mode: AdmissionMode) -> NewCallAdmission {
        NewCallAdmission::new(NewCallAdmissionConfig::new(mode))
    }

    #[test]
    fn overload_response_copies_every_via_in_order_and_tags_to() {
        let request = invite(None);
        let response = build_overload_response(&request, Some(12));
        assert_eq!(response.status(), StatusCode::ServiceUnavailable);
        let vias: Vec<_> = response
            .headers
            .iter()
            .filter_map(|header| match header {
                TypedHeader::Via(via) => via.branch().map(str::to_owned),
                _ => None,
            })
            .collect();
        assert_eq!(vias, ["z9hG4bKproxy1", "z9hG4bKorigin1"]);
        let tag = response.to().and_then(|to| to.tag()).expect("To tag");
        assert!(tag.starts_with(OVERLOAD_TO_TAG_PREFIX));
        assert_eq!(response.call_id().unwrap().as_str(), "call-1@example.com");
        assert_eq!(
            response.from().and_then(|from| from.tag()),
            Some("fromtag1")
        );
        let retry_after = response
            .header(&HeaderName::RetryAfter)
            .expect("Retry-After")
            .to_string();
        assert!(retry_after.contains("12"), "{retry_after}");
        assert!(response.header(&HeaderName::ContentLength).is_some());
        assert!(response.header(&HeaderName::RecordRoute).is_none());
        assert!(response.header(&HeaderName::Contact).is_none());
    }

    #[test]
    fn retransmission_gets_identical_tag_and_retry_after_can_be_omitted() {
        let first = build_overload_response(&invite(None), None);
        let second = build_overload_response(&invite(None), None);
        assert_eq!(
            first.to().and_then(|to| to.tag()),
            second.to().and_then(|to| to.tag())
        );
        assert!(first.header(&HeaderName::RetryAfter).is_none());
    }

    #[test]
    fn classification_only_targets_new_invites_and_our_acks() {
        assert!(is_new_dialog_invite(&invite(None)));
        assert!(!is_new_dialog_invite(&invite(Some("remote"))));
        let reject = build_overload_response(&invite(None), None);
        assert!(is_overload_reject_ack(&ack_for(&reject)));
        let foreign = build_overload_response(&invite(Some("remote")), None);
        assert!(!is_overload_reject_ack(&ack_for(&foreign)));
    }

    #[test]
    fn rate_limit_admits_burst_then_rejects_until_tokens_return() {
        let gate = NewCallAdmission::new(
            NewCallAdmissionConfig::new(AdmissionMode::Enforce)
                .with_rate_limit(NewCallRateLimit::new(1000.0, 5.0)),
        );
        let admitted = (0..50)
            .filter(|i| gate.decide(&format!("c{i}")) == AdmissionDecision::Admit)
            .count();
        assert!((5..=8).contains(&admitted), "burst admitted {admitted}");
        let stats = gate.stats();
        assert!(stats.rate_limited > 0);
        assert_eq!(stats.admitted(), admitted as u64);
        assert_eq!(stats.rate_limit, Some(NewCallRateLimit::new(1000.0, 5.0)));
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(gate.decide("later"), AdmissionDecision::Admit);
        gate.set_rate_limit(None);
        assert_eq!(gate.stats().rate_limit, None);
        assert!((0..100).all(|i| gate.decide(&format!("d{i}")) == AdmissionDecision::Admit));
        gate.set_rate_limit(Some(NewCallRateLimit::new(f64::NAN, 1.0)));
        assert_eq!(gate.stats().rate_limit, None);
    }

    #[test]
    fn gate_modes_and_deterministic_shedding() {
        let gate = gate(AdmissionMode::Off);
        gate.set_level(AdmissionLevel::closed(Some(20)));
        assert_eq!(gate.decide("a"), AdmissionDecision::Admit);
        assert_eq!(gate.stats().decisions, 0, "Off must not count decisions");

        gate.set_mode(AdmissionMode::Observe);
        assert_eq!(gate.decide("a"), AdmissionDecision::Admit);
        assert_eq!(gate.stats().would_reject, 1);

        gate.set_mode(AdmissionMode::Enforce);
        assert_eq!(
            gate.decide("a"),
            AdmissionDecision::Reject {
                retry_after_secs: Some(20)
            }
        );

        gate.set_level(AdmissionLevel::shedding(500_000, None));
        let first: Vec<_> = (0..200)
            .map(|i| gate.decide(&format!("call-{i}")))
            .collect();
        let second: Vec<_> = (0..200)
            .map(|i| gate.decide(&format!("call-{i}")))
            .collect();
        assert_eq!(first, second);
        let rejected = first
            .iter()
            .filter(|decision| {
                matches!(
                    decision,
                    AdmissionDecision::Reject {
                        retry_after_secs: None
                    }
                )
            })
            .count();
        assert!(
            (60..=140).contains(&rejected),
            "shed fraction far from 50%: {rejected}"
        );

        gate.set_level(AdmissionLevel::open());
        assert_eq!(gate.decide("call-1"), AdmissionDecision::Admit);
        assert_eq!(gate.level(), AdmissionLevel::open());
    }

    #[test]
    fn independent_gates_do_not_share_state() {
        let closed = gate(AdmissionMode::Enforce);
        closed.set_level(AdmissionLevel::closed(None));
        let open = gate(AdmissionMode::Enforce);
        assert!(matches!(
            closed.decide("x"),
            AdmissionDecision::Reject { .. }
        ));
        assert_eq!(open.decide("x"), AdmissionDecision::Admit);
        assert_eq!(open.stats().rejected, 0);
    }

    #[test]
    fn tls_ingress_verdict_answers_new_invites_absorbs_our_acks_and_passes_the_rest() {
        let gate = Arc::new(gate(AdmissionMode::Enforce));
        assert!(!gate.stats().tls_ingress_attached);
        let _hook = gate.tls_ingress_hook();
        assert!(gate.stats().tls_ingress_attached);

        // Open gate: the INVITE passes, in-dialog requests are never decided.
        assert!(matches!(
            gate.ingress_verdict(&Message::Request(invite(None))),
            IngressVerdict::Pass
        ));
        gate.set_level(AdmissionLevel::closed(Some(9)));
        assert!(matches!(
            gate.ingress_verdict(&Message::Request(invite(Some("remote")))),
            IngressVerdict::Pass
        ));

        let IngressVerdict::Respond(bytes) = gate.ingress_verdict(&Message::Request(invite(None)))
        else {
            panic!("a closed gate must answer a new INVITE");
        };
        let Message::Response(response) = rvoip_sip_core::parse_message(&bytes).expect("parse 503")
        else {
            panic!("expected a response");
        };
        assert_eq!(response.status(), StatusCode::ServiceUnavailable);
        assert!(response
            .header(&HeaderName::RetryAfter)
            .is_some_and(|header| header.to_string().contains('9')));

        assert!(matches!(
            gate.ingress_verdict(&Message::Request(ack_for(&response))),
            IngressVerdict::Absorb
        ));
        let stats = gate.stats();
        assert_eq!(stats.tls_ingress_rejected, 1);
        assert_eq!(stats.rejected, 1);
        assert_eq!(stats.acks_absorbed, 1);
    }
}
