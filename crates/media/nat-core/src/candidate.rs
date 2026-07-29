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

    /// Parse the `typ` token from an SDP `a=candidate:` line (RFC 8839
    /// §5.1.1) — the reverse of [`Self::as_str`].
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "host" => Some(CandidateKind::Host),
            "srflx" => Some(CandidateKind::ServerReflexive),
            "prflx" => Some(CandidateKind::PeerReflexive),
            "relay" => Some(CandidateKind::Relay),
            _ => None,
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

impl IceCandidate {
    /// RFC 8839 §5.1 wire text for this candidate, without the leading
    /// `a=candidate:` attribute prefix — callers building SDP (e.g.
    /// `rvoip-sip` via `sip-core`'s `MediaBuilder::ice_candidate`) pass
    /// this straight through; [`crate::agent::IceAgent::add_remote_candidate`]
    /// parses the same shape back.
    pub fn to_sdp_line(&self) -> String {
        let mut s = format!(
            "{} {} {} {} {} {} typ {}",
            self.foundation,
            self.component,
            self.transport,
            self.priority,
            self.address,
            self.port,
            self.kind.as_str()
        );
        if let (Some(addr), Some(port)) = (self.related_address, self.related_port) {
            s.push_str(&format!(" raddr {addr} rport {port}"));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_kind_parse_is_the_reverse_of_as_str() {
        for kind in [
            CandidateKind::Host,
            CandidateKind::ServerReflexive,
            CandidateKind::PeerReflexive,
            CandidateKind::Relay,
        ] {
            assert_eq!(CandidateKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(CandidateKind::parse("bogus"), None);
    }

    #[test]
    fn to_sdp_line_round_trips_through_a_host_candidate() {
        let candidate = IceCandidate {
            foundation: "1".to_string(),
            component: 1,
            transport: "udp".to_string(),
            priority: 2130706431,
            address: "127.0.0.1".parse().unwrap(),
            port: 5000,
            kind: CandidateKind::Host,
            related_address: None,
            related_port: None,
        };
        assert_eq!(
            candidate.to_sdp_line(),
            "1 1 udp 2130706431 127.0.0.1 5000 typ host"
        );
    }

    #[test]
    fn to_sdp_line_includes_raddr_rport_for_server_reflexive() {
        let candidate = IceCandidate {
            foundation: "2".to_string(),
            component: 1,
            transport: "udp".to_string(),
            priority: 1694498815,
            address: "203.0.113.1".parse().unwrap(),
            port: 6000,
            kind: CandidateKind::ServerReflexive,
            related_address: Some("192.168.1.1".parse().unwrap()),
            related_port: Some(5000),
        };
        assert_eq!(
            candidate.to_sdp_line(),
            "2 1 udp 1694498815 203.0.113.1 6000 typ srflx raddr 192.168.1.1 rport 5000"
        );
    }
}
