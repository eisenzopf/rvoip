//! RFC 8839 candidate representation, independent of any SDP text form.
//!
//! Building/parsing the actual `a=candidate:` attribute text is the
//! caller's job (e.g. `rvoip-sip`, via `sip-core`'s existing typed
//! `CandidateAttribute`) — same SDP-agnostic boundary already used by
//! `rvoip-rtp-core::dtls_srtp` for the DTLS-SRTP handshake result.

use std::net::IpAddr;

/// RFC 8839 §5.1.1 candidate type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateKind {
    Host,
    ServerReflexive,
    PeerReflexive,
    Relay,
}

impl CandidateKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CandidateKind::Host => "host",
            CandidateKind::ServerReflexive => "srflx",
            CandidateKind::PeerReflexive => "prflx",
            CandidateKind::Relay => "relay",
        }
    }
}

/// The fields an SDP `a=candidate:` line needs (RFC 8839 §5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IceCandidate {
    pub foundation: String,
    pub component: u32,
    /// Always `"udp"` for everything this crate gathers.
    pub transport: String,
    pub priority: u32,
    pub address: IpAddr,
    pub port: u16,
    pub kind: CandidateKind,
    pub related_address: Option<IpAddr>,
    pub related_port: Option<u16>,
}
