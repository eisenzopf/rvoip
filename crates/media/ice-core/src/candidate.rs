//! ICE candidates and the RFC 8445 §5.1.2 priority arithmetic.

use std::hash::{Hash, Hasher};
use std::net::SocketAddr;

/// How a candidate was learned.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum CandidateKind {
    /// A local interface address.
    Host,
    /// Our address as a STUN server saw it.
    ServerReflexive,
    /// Learned from the source address of a peer's connectivity check.
    PeerReflexive,
    /// Allocated on a TURN relay (modeled; gathering is a later phase).
    Relayed,
}

impl CandidateKind {
    /// RFC 8445 §5.1.2.2 recommended type preferences.
    #[must_use]
    pub const fn type_preference(self) -> u32 {
        match self {
            Self::Host => 126,
            Self::PeerReflexive => 110,
            Self::ServerReflexive => 100,
            Self::Relayed => 0,
        }
    }

    /// The SDP `typ` token (RFC 8839 §5.1).
    #[must_use]
    pub const fn sdp_type(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::ServerReflexive => "srflx",
            Self::PeerReflexive => "prflx",
            Self::Relayed => "relay",
        }
    }

    /// Parse the SDP `typ` token.
    #[must_use]
    pub fn from_sdp_type(token: &str) -> Option<Self> {
        match token {
            "host" => Some(Self::Host),
            "srflx" => Some(Self::ServerReflexive),
            "prflx" => Some(Self::PeerReflexive),
            "relay" => Some(Self::Relayed),
            _ => None,
        }
    }
}

/// One ICE candidate. UDP only: rvoip's SIP media is UDP, and TCP candidate
/// types (RFC 6544) are an explicit non-goal of the plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Candidate {
    /// Groups candidates that would behave identically for checks: same
    /// kind, same base, same STUN server. Used by the unfreezing rules.
    pub foundation: String,
    /// ICE component: 1 = RTP. (RTCP is component 2; v1 requires rtcp-mux,
    /// so only component 1 is ever gathered, but the field stays so
    /// two-component support is an extension rather than a remodel.)
    pub component: u8,
    /// RFC 8445 §5.1.2.1 priority.
    pub priority: u32,
    /// The transport address peers should send to.
    pub addr: SocketAddr,
    /// How this candidate was learned.
    pub kind: CandidateKind,
    /// The local socket this candidate is reached through. Equal to `addr`
    /// for host candidates; the NAT-inside address for reflexive ones.
    pub base: SocketAddr,
    /// The related address for SDP (`raddr`/`rport`): the base for srflx.
    pub related: Option<SocketAddr>,
}

impl Candidate {
    /// A host candidate on a local socket.
    #[must_use]
    pub fn host(addr: SocketAddr, component: u8, local_preference: u16) -> Self {
        Self {
            foundation: foundation(CandidateKind::Host, addr, None),
            component,
            priority: priority(CandidateKind::Host, local_preference, component),
            addr,
            kind: CandidateKind::Host,
            base: addr,
            related: None,
        }
    }

    /// A server-reflexive candidate: `mapped` as a STUN server saw us,
    /// reached through the local socket `base`.
    #[must_use]
    pub fn server_reflexive(
        mapped: SocketAddr,
        base: SocketAddr,
        stun_server: SocketAddr,
        component: u8,
        local_preference: u16,
    ) -> Self {
        Self {
            foundation: foundation(CandidateKind::ServerReflexive, base, Some(stun_server)),
            component,
            priority: priority(CandidateKind::ServerReflexive, local_preference, component),
            addr: mapped,
            kind: CandidateKind::ServerReflexive,
            base,
            related: Some(base),
        }
    }

    /// A peer-reflexive candidate learned from an inbound check's source.
    #[must_use]
    pub fn peer_reflexive(addr: SocketAddr, base: SocketAddr, component: u8, priority: u32) -> Self {
        Self {
            foundation: foundation(CandidateKind::PeerReflexive, addr, None),
            component,
            priority,
            addr,
            kind: CandidateKind::PeerReflexive,
            base,
            related: None,
        }
    }
}

/// RFC 8445 §5.1.2.1: `(2^24)·type + (2^8)·local + (256 − component)`.
#[must_use]
pub fn priority(kind: CandidateKind, local_preference: u16, component: u8) -> u32 {
    (kind.type_preference() << 24)
        + (u32::from(local_preference) << 8)
        + (256 - u32::from(component))
}

/// The priority a peer-reflexive candidate discovered by our checks would
/// have — carried in every check's PRIORITY attribute (RFC 8445 §7.1.1).
#[must_use]
pub fn prflx_priority(local_preference: u16, component: u8) -> u32 {
    priority(CandidateKind::PeerReflexive, local_preference, component)
}

/// RFC 8445 §6.1.2.3 pair priority. `g` is the controlling agent's candidate
/// priority, `d` the controlled agent's.
#[must_use]
pub fn pair_priority(g: u32, d: u32) -> u64 {
    let (min, max) = (u64::from(g.min(d)), u64::from(g.max(d)));
    (min << 32) + (max << 1) + u64::from(g > d)
}

/// Foundations only need to be equal for candidates that would behave the
/// same, within one agent — a short stable hash serves.
fn foundation(kind: CandidateKind, base: SocketAddr, server: Option<SocketAddr>) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    kind.hash(&mut hasher);
    base.ip().hash(&mut hasher);
    server.map(|address| address.ip()).hash(&mut hasher);
    format!("{:08x}", hasher.finish() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priorities_follow_the_type_ladder() {
        let host = Candidate::host("10.0.0.1:5004".parse().unwrap(), 1, 65_535);
        let srflx = Candidate::server_reflexive(
            "198.51.100.1:6000".parse().unwrap(),
            "10.0.0.1:5004".parse().unwrap(),
            "198.51.100.99:3478".parse().unwrap(),
            1,
            65_535,
        );
        assert!(host.priority > srflx.priority, "host outranks srflx");
        // Worked example from the formula: host, max local pref, component 1.
        assert_eq!(host.priority, (126 << 24) + (65_535 << 8) + 255);
    }

    #[test]
    fn pair_priority_orders_and_breaks_ties_by_controlling_side() {
        // Same {G,D} set must produce the same min/max core either way
        // around, differing only in the G>D tie bit.
        let forward = pair_priority(200, 100);
        let backward = pair_priority(100, 200);
        assert_eq!(forward >> 1, backward >> 1);
        assert_eq!(forward & 1, 1);
        assert_eq!(backward & 1, 0);
        assert!(pair_priority(200, 199) > pair_priority(100, 300).min(pair_priority(300, 100)));
    }

    #[test]
    fn foundations_group_like_candidates_and_split_unlike_ones() {
        let a = Candidate::host("10.0.0.1:5004".parse().unwrap(), 1, 65_535);
        let b = Candidate::host("10.0.0.1:5006".parse().unwrap(), 1, 65_534);
        let c = Candidate::host("10.0.0.2:5004".parse().unwrap(), 1, 65_535);
        assert_eq!(a.foundation, b.foundation, "same kind + base ip group");
        assert_ne!(a.foundation, c.foundation, "different base ip splits");
    }
}
