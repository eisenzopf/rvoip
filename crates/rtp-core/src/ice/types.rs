//! Core types for the ICE (Interactive Connectivity Establishment) implementation.
//!
//! Defines candidates, candidate pairs, credentials, and state enumerations
//! per RFC 8445.

use std::fmt;
use std::net::SocketAddr;

/// ICE candidate type per RFC 8445 Section 5.1.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CandidateType {
    /// Host candidate: address obtained directly from a local interface.
    Host,
    /// Server-reflexive candidate: address discovered via STUN.
    ServerReflexive,
    /// Peer-reflexive candidate: address discovered during connectivity checks.
    PeerReflexive,
    /// Relay candidate: address allocated on a TURN server.
    Relay,
}

impl CandidateType {
    /// Type preference value per RFC 8445 Section 5.1.2.1.
    pub fn type_preference(self) -> u32 {
        match self {
            Self::Host => 126,
            Self::PeerReflexive => 110,
            Self::ServerReflexive => 100,
            Self::Relay => 0,
        }
    }
}

impl fmt::Display for CandidateType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Host => write!(f, "host"),
            Self::ServerReflexive => write!(f, "srflx"),
            Self::PeerReflexive => write!(f, "prflx"),
            Self::Relay => write!(f, "relay"),
        }
    }
}

/// ICE agent role per RFC 8445 Section 6.1.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IceRole {
    /// Controlling agent: makes nomination decisions.
    Controlling,
    /// Controlled agent: follows the controlling agent's nominations.
    Controlled,
}

impl fmt::Display for IceRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Controlling => write!(f, "controlling"),
            Self::Controlled => write!(f, "controlled"),
        }
    }
}

/// ICE connection state per RFC 8445.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IceConnectionState {
    /// Initial state, no activity.
    New,
    /// Gathering local candidates.
    Gathering,
    /// Performing connectivity checks.
    Checking,
    /// At least one candidate pair has succeeded.
    Connected,
    /// All checks completed, a nominated pair is selected.
    Completed,
    /// All candidate pairs have failed.
    Failed,
    /// Connectivity was lost after being established.
    Disconnected,
    /// The agent has been shut down.
    Closed,
}

impl fmt::Display for IceConnectionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::New => write!(f, "new"),
            Self::Gathering => write!(f, "gathering"),
            Self::Checking => write!(f, "checking"),
            Self::Connected => write!(f, "connected"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Disconnected => write!(f, "disconnected"),
            Self::Closed => write!(f, "closed"),
        }
    }
}

/// RTP/RTCP component identifier per RFC 8445 Section 4.1.1.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ComponentId {
    /// RTP component.
    Rtp = 1,
    /// RTCP component.
    Rtcp = 2,
}

impl ComponentId {
    /// Return the numeric component ID.
    pub fn id(self) -> u32 {
        self as u32
    }
}

impl fmt::Display for ComponentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rtp => write!(f, "RTP(1)"),
            Self::Rtcp => write!(f, "RTCP(2)"),
        }
    }
}

/// An ICE candidate representing a potential transport address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IceCandidate {
    /// Foundation string identifying similar candidates (RFC 8445 Section 5.1.1.3).
    pub foundation: String,
    /// Component identifier (RTP or RTCP).
    pub component: ComponentId,
    /// Transport protocol (typically "udp").
    pub transport: String,
    /// Candidate priority (RFC 8445 Section 5.1.2).
    pub priority: u32,
    /// The transport address.
    pub address: SocketAddr,
    /// Candidate type.
    pub candidate_type: CandidateType,
    /// Related address for server-reflexive and relay candidates.
    pub related_address: Option<SocketAddr>,
    /// ICE ufrag this candidate belongs to.
    pub ufrag: String,
}

impl IceCandidate {
    /// Format this candidate as an SDP `a=candidate:` attribute value.
    pub fn to_sdp_attribute(&self) -> String {
        let mut s = format!(
            "{} {} {} {} {} {} typ {}",
            self.foundation,
            self.component.id(),
            self.transport,
            self.priority,
            self.address.ip(),
            self.address.port(),
            self.candidate_type,
        );
        if let Some(ref raddr) = self.related_address {
            s.push_str(&format!(" raddr {} rport {}", raddr.ip(), raddr.port()));
        }
        s
    }
}

impl std::str::FromStr for CandidateType {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "host" => Ok(Self::Host),
            "srflx" => Ok(Self::ServerReflexive),
            "prflx" => Ok(Self::PeerReflexive),
            "relay" => Ok(Self::Relay),
            other => Err(format!("unknown candidate type: {}", other)),
        }
    }
}

impl IceCandidate {
    /// Parse an ICE candidate from an SDP attribute value.
    ///
    /// Accepts the value portion of `a=candidate:...` (without the
    /// `a=candidate:` prefix) as well as the full attribute line.
    ///
    /// Format per RFC 8445 / RFC 8840:
    /// ```text
    /// foundation component transport priority address port typ type [raddr addr rport port]
    /// ```
    pub fn from_sdp_attribute(line: &str) -> std::result::Result<Self, String> {
        // Strip optional "a=candidate:" prefix
        let value = line.strip_prefix("a=candidate:").unwrap_or(line);
        let value = value.trim();
        let parts: Vec<&str> = value.split_whitespace().collect();

        // Minimum 8 tokens: foundation component transport priority addr port "typ" type
        if parts.len() < 8 {
            return Err(format!(
                "candidate attribute too short ({} tokens, need >= 8): {}",
                parts.len(),
                value
            ));
        }

        let foundation = parts[0].to_string();

        let component_id: u8 = parts[1]
            .parse()
            .map_err(|e| format!("invalid component id '{}': {}", parts[1], e))?;
        let component = match component_id {
            1 => ComponentId::Rtp,
            2 => ComponentId::Rtcp,
            other => return Err(format!("unsupported component id: {}", other)),
        };

        let transport = parts[2].to_lowercase();

        let priority: u32 = parts[3]
            .parse()
            .map_err(|e| format!("invalid priority '{}': {}", parts[3], e))?;

        let ip: std::net::IpAddr = parts[4]
            .parse()
            .map_err(|e| format!("invalid IP address '{}': {}", parts[4], e))?;
        let port: u16 = parts[5]
            .parse()
            .map_err(|e| format!("invalid port '{}': {}", parts[5], e))?;
        let address = SocketAddr::new(ip, port);

        // parts[6] should be "typ"
        if parts[6] != "typ" {
            return Err(format!("expected 'typ' keyword at position 6, got '{}'", parts[6]));
        }

        let candidate_type: CandidateType = parts[7]
            .parse()
            .map_err(|e| format!("invalid candidate type: {}", e))?;

        // Parse optional raddr/rport
        let mut related_address = None;
        let mut i = 8;
        while i + 1 < parts.len() {
            match parts[i] {
                "raddr" => {
                    let raddr_ip: std::net::IpAddr = parts[i + 1]
                        .parse()
                        .map_err(|e| format!("invalid raddr '{}': {}", parts[i + 1], e))?;
                    // Look for rport
                    if i + 3 < parts.len() && parts[i + 2] == "rport" {
                        let rport: u16 = parts[i + 3]
                            .parse()
                            .map_err(|e| format!("invalid rport '{}': {}", parts[i + 3], e))?;
                        related_address = Some(SocketAddr::new(raddr_ip, rport));
                        i += 4;
                    } else {
                        related_address = Some(SocketAddr::new(raddr_ip, 0));
                        i += 2;
                    }
                }
                _ => {
                    // Skip unknown extensions
                    i += 2;
                }
            }
        }

        Ok(Self {
            foundation,
            component,
            transport,
            priority,
            address,
            candidate_type,
            related_address,
            ufrag: String::new(), // ufrag is set separately from SDP session attributes
        })
    }
}

impl fmt::Display for IceCandidate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}({} {} {}:{})",
            self.candidate_type,
            self.foundation,
            self.transport,
            self.address.ip(),
            self.address.port(),
        )
    }
}

/// State of a candidate pair in the checklist per RFC 8445 Section 6.1.2.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidatePairState {
    /// Check has not been performed and it is not yet triggered.
    Frozen,
    /// Check has not been performed but is scheduled to be triggered.
    Waiting,
    /// Connectivity check is in-progress.
    InProgress,
    /// Check succeeded.
    Succeeded,
    /// Check failed.
    Failed,
}

impl fmt::Display for CandidatePairState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Frozen => write!(f, "frozen"),
            Self::Waiting => write!(f, "waiting"),
            Self::InProgress => write!(f, "in-progress"),
            Self::Succeeded => write!(f, "succeeded"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

/// A candidate pair consisting of a local and remote candidate.
#[derive(Debug, Clone)]
pub struct IceCandidatePair {
    /// Local candidate.
    pub local: IceCandidate,
    /// Remote candidate.
    pub remote: IceCandidate,
    /// Current state of the pair.
    pub state: CandidatePairState,
    /// Pair priority (RFC 8445 Section 6.1.2.3).
    pub priority: u64,
    /// Whether this pair has been nominated.
    pub nominated: bool,
}

impl IceCandidatePair {
    /// Compute pair priority per RFC 8445 Section 6.1.2.3.
    ///
    /// pair_priority = 2^32 * MIN(G,D) + 2 * MAX(G,D) + (G > D ? 1 : 0)
    /// where G = controlling candidate priority, D = controlled candidate priority.
    pub fn compute_priority(controlling_prio: u32, controlled_prio: u32) -> u64 {
        let g = controlling_prio as u64;
        let d = controlled_prio as u64;
        let min = g.min(d);
        let max = g.max(d);
        let tie = if g > d { 1u64 } else { 0u64 };
        (1u64 << 32) * min + 2 * max + tie
    }
}

impl fmt::Display for IceCandidatePair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{} <-> {} state={} prio={} nom={}]",
            self.local, self.remote, self.state, self.priority, self.nominated
        )
    }
}

/// ICE credentials (ufrag and password) for authentication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IceCredentials {
    /// ICE username fragment (4+ characters, per RFC 8445).
    pub ufrag: String,
    /// ICE password (22+ characters, per RFC 8445).
    pub pwd: String,
}

impl IceCredentials {
    /// Generate new random ICE credentials.
    ///
    /// Produces a 4-character ufrag and 22-character password using
    /// alphanumeric characters (ICE-chars per RFC 8445 Section 5.3).
    pub fn generate() -> Self {
        use rand::Rng;
        const ICE_CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789+/";

        let mut rng = rand::thread_rng();

        let ufrag: String = (0..4)
            .map(|_| {
                let idx = rng.gen_range(0..ICE_CHARS.len());
                ICE_CHARS[idx] as char
            })
            .collect();

        let pwd: String = (0..22)
            .map(|_| {
                let idx = rng.gen_range(0..ICE_CHARS.len());
                ICE_CHARS[idx] as char
            })
            .collect();

        Self { ufrag, pwd }
    }
}

impl fmt::Display for IceCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ufrag={} pwd=<{} chars>", self.ufrag, self.pwd.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_candidate_type_preference() {
        assert_eq!(CandidateType::Host.type_preference(), 126);
        assert_eq!(CandidateType::PeerReflexive.type_preference(), 110);
        assert_eq!(CandidateType::ServerReflexive.type_preference(), 100);
        assert_eq!(CandidateType::Relay.type_preference(), 0);
    }

    #[test]
    fn test_candidate_type_display() {
        assert_eq!(format!("{}", CandidateType::Host), "host");
        assert_eq!(format!("{}", CandidateType::ServerReflexive), "srflx");
        assert_eq!(format!("{}", CandidateType::PeerReflexive), "prflx");
        assert_eq!(format!("{}", CandidateType::Relay), "relay");
    }

    #[test]
    fn test_credentials_generation() {
        let creds = IceCredentials::generate();
        assert_eq!(creds.ufrag.len(), 4);
        assert_eq!(creds.pwd.len(), 22);

        // Verify uniqueness
        let creds2 = IceCredentials::generate();
        assert_ne!(creds.ufrag, creds2.ufrag);
    }

    #[test]
    fn test_pair_priority_computation() {
        // RFC 8445 example: controlling prio > controlled prio
        let prio = IceCandidatePair::compute_priority(1000, 500);
        // 2^32 * 500 + 2 * 1000 + 1 = 2147483648000 + 2000 + 1
        let expected = (1u64 << 32) * 500 + 2 * 1000 + 1;
        assert_eq!(prio, expected);

        // Equal priorities
        let prio_eq = IceCandidatePair::compute_priority(100, 100);
        let expected_eq = (1u64 << 32) * 100 + 2 * 100 + 0;
        assert_eq!(prio_eq, expected_eq);
    }

    #[test]
    fn test_component_id() {
        assert_eq!(ComponentId::Rtp.id(), 1);
        assert_eq!(ComponentId::Rtcp.id(), 2);
    }

    #[test]
    fn test_candidate_sdp_attribute() {
        let candidate = IceCandidate {
            foundation: "1".to_string(),
            component: ComponentId::Rtp,
            transport: "udp".to_string(),
            priority: 2130706431,
            address: "192.168.1.100:5000".parse().unwrap_or_else(|e| panic!("parse: {e}")),
            candidate_type: CandidateType::Host,
            related_address: None,
            ufrag: "abcd".to_string(),
        };
        let sdp = candidate.to_sdp_attribute();
        assert!(sdp.contains("1 1 udp 2130706431"));
        assert!(sdp.contains("typ host"));
    }

    #[test]
    fn test_candidate_sdp_attribute_with_related() {
        let candidate = IceCandidate {
            foundation: "2".to_string(),
            component: ComponentId::Rtp,
            transport: "udp".to_string(),
            priority: 1694498815,
            address: "203.0.113.5:12345".parse().unwrap_or_else(|e| panic!("parse: {e}")),
            candidate_type: CandidateType::ServerReflexive,
            related_address: Some("192.168.1.100:5000".parse().unwrap_or_else(|e| panic!("parse: {e}"))),
            ufrag: "abcd".to_string(),
        };
        let sdp = candidate.to_sdp_attribute();
        assert!(sdp.contains("typ srflx"));
        assert!(sdp.contains("raddr 192.168.1.100 rport 5000"));
    }

    #[test]
    fn test_connection_state_display() {
        assert_eq!(format!("{}", IceConnectionState::New), "new");
        assert_eq!(format!("{}", IceConnectionState::Checking), "checking");
        assert_eq!(format!("{}", IceConnectionState::Completed), "completed");
    }

    #[test]
    fn test_pair_state_display() {
        assert_eq!(format!("{}", CandidatePairState::Frozen), "frozen");
        assert_eq!(format!("{}", CandidatePairState::Succeeded), "succeeded");
    }

    // --- IceCandidate::from_sdp_attribute (trickle ICE) tests ---

    #[test]
    fn test_parse_host_candidate() {
        let line = "1 1 udp 2130706431 192.168.1.100 5000 typ host";
        let c = IceCandidate::from_sdp_attribute(line)
            .unwrap_or_else(|e| panic!("parse: {e}"));
        assert_eq!(c.foundation, "1");
        assert_eq!(c.component, ComponentId::Rtp);
        assert_eq!(c.transport, "udp");
        assert_eq!(c.priority, 2130706431);
        assert_eq!(
            c.address,
            "192.168.1.100:5000"
                .parse::<SocketAddr>()
                .unwrap_or_else(|e| panic!("parse: {e}"))
        );
        assert_eq!(c.candidate_type, CandidateType::Host);
        assert!(c.related_address.is_none());
    }

    #[test]
    fn test_parse_srflx_candidate_with_related() {
        let line = "2 1 udp 1694498815 203.0.113.5 12345 typ srflx raddr 192.168.1.100 rport 5000";
        let c = IceCandidate::from_sdp_attribute(line)
            .unwrap_or_else(|e| panic!("parse: {e}"));
        assert_eq!(c.candidate_type, CandidateType::ServerReflexive);
        assert_eq!(
            c.related_address,
            Some(
                "192.168.1.100:5000"
                    .parse::<SocketAddr>()
                    .unwrap_or_else(|e| panic!("parse: {e}"))
            )
        );
    }

    #[test]
    fn test_parse_candidate_with_a_prefix() {
        let line = "a=candidate:1 1 udp 2130706431 10.0.0.1 6000 typ host";
        let c = IceCandidate::from_sdp_attribute(line)
            .unwrap_or_else(|e| panic!("parse: {e}"));
        assert_eq!(c.foundation, "1");
        assert_eq!(c.candidate_type, CandidateType::Host);
    }

    #[test]
    fn test_parse_relay_candidate() {
        let line = "3 1 udp 16777215 198.51.100.1 54321 typ relay raddr 203.0.113.5 rport 12345";
        let c = IceCandidate::from_sdp_attribute(line)
            .unwrap_or_else(|e| panic!("parse: {e}"));
        assert_eq!(c.candidate_type, CandidateType::Relay);
        assert!(c.related_address.is_some());
    }

    #[test]
    fn test_parse_candidate_too_short() {
        let line = "1 1 udp";
        let result = IceCandidate::from_sdp_attribute(line);
        assert!(result.is_err());
    }

    #[test]
    fn test_roundtrip_sdp_attribute() {
        let original = IceCandidate {
            foundation: "42".to_string(),
            component: ComponentId::Rtp,
            transport: "udp".to_string(),
            priority: 100,
            address: "10.0.0.1:9000"
                .parse()
                .unwrap_or_else(|e| panic!("parse: {e}")),
            candidate_type: CandidateType::Host,
            related_address: None,
            ufrag: "test".to_string(),
        };
        let sdp = original.to_sdp_attribute();
        let parsed = IceCandidate::from_sdp_attribute(&sdp)
            .unwrap_or_else(|e| panic!("parse: {e}"));
        assert_eq!(parsed.foundation, original.foundation);
        assert_eq!(parsed.priority, original.priority);
        assert_eq!(parsed.address, original.address);
        assert_eq!(parsed.candidate_type, original.candidate_type);
    }

    #[test]
    fn test_candidate_type_from_str() {
        assert_eq!("host".parse::<CandidateType>().unwrap_or_else(|e| panic!("{e}")), CandidateType::Host);
        assert_eq!("srflx".parse::<CandidateType>().unwrap_or_else(|e| panic!("{e}")), CandidateType::ServerReflexive);
        assert_eq!("prflx".parse::<CandidateType>().unwrap_or_else(|e| panic!("{e}")), CandidateType::PeerReflexive);
        assert_eq!("relay".parse::<CandidateType>().unwrap_or_else(|e| panic!("{e}")), CandidateType::Relay);
        assert!("unknown".parse::<CandidateType>().is_err());
    }
}
