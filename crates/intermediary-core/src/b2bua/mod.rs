//! B2BUA (Back-to-Back User Agent) implementation
//!
//! A B2BUA terminates SIP dialogs on both sides and bridges them internally.
//! Per RFC 3261, the B2BUA acts as a UAS on the inbound leg and a UAC on the
//! outbound leg, generating fresh headers (Call-ID, Via, From tag) for the
//! outbound dialog while maintaining a mapping between the two legs.
//!
//! # Architecture
//!
//! ```text
//! Caller (Leg A) <--SIP--> B2BUA <--SIP--> Callee (Leg B)
//!                    UAS side    UAC side
//!
//! Media:
//! Caller RTP -----> B2BUA relay -----> Callee RTP
//! Caller RTP <----- B2BUA relay <----- Callee RTP
//! ```

use crate::common::{errors::Result, types::*};
use rvoip_session_core::api::bridge::CallBridge;
use rvoip_session_core::api::call::SimpleCall;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// State of a B2BUA session (the paired inbound + outbound legs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum B2buaSessionState {
    /// Outbound INVITE has been sent; waiting for answer.
    SettingUp,
    /// Both legs are established and media is bridged.
    Active,
    /// BYE is being sent on both legs.
    Terminating,
    /// Both legs have been torn down.
    Terminated,
}

/// Represents the two legs of a B2BUA bridged call.
pub struct B2buaSession {
    /// Unique id for this paired session.
    pub id: IntermediarySessionId,
    /// Inbound leg (UAS side – toward the caller).
    pub inbound_call: SimpleCall,
    /// Outbound leg (UAC side – toward the callee).
    pub outbound_call: SimpleCall,
    /// Current state of the session pair.
    pub state: B2buaSessionState,
    /// Bridge connecting media between the two legs.
    pub bridge: Option<CallBridge>,
    /// Original SDP offer received on leg A (for re-INVITE forwarding).
    pub inbound_sdp: Option<String>,
    /// SDP answer from leg B.
    pub outbound_sdp: Option<String>,
}

/// Header manipulation rules applied between legs.
#[derive(Debug, Clone)]
pub struct HeaderManipulation {
    /// Headers to add on the outbound leg.
    pub add_headers: Vec<(String, String)>,
    /// Header names to strip when forwarding from inbound to outbound.
    pub strip_headers: Vec<String>,
    /// Headers whose values should be rewritten (name -> new value).
    pub rewrite_headers: Vec<(String, String)>,
}

impl Default for HeaderManipulation {
    fn default() -> Self {
        Self {
            add_headers: Vec::new(),
            // Per RFC 3261 the B2BUA must generate fresh Via and Record-Route.
            strip_headers: vec![
                "Via".to_string(),
                "Record-Route".to_string(),
                "Route".to_string(),
            ],
            rewrite_headers: Vec::new(),
        }
    }
}

/// Configuration for the B2BUA engine.
#[derive(Debug, Clone)]
pub struct B2buaConfig {
    /// Identity used for outbound legs (appears in From header).
    pub identity: String,
    /// Local IP address for SDP.
    pub local_ip: String,
    /// Base port for the outbound peer (will try successive ports if busy).
    pub base_port: u16,
    /// Header manipulation rules.
    pub header_rules: HeaderManipulation,
    /// Media port range start for the B2BUA's own RTP endpoints.
    pub media_port_start: u16,
    /// Media port range end.
    pub media_port_end: u16,
}

impl Default for B2buaConfig {
    fn default() -> Self {
        Self {
            identity: "b2bua".to_string(),
            local_ip: "127.0.0.1".to_string(),
            base_port: 5080,
            header_rules: HeaderManipulation::default(),
            media_port_start: 30000,
            media_port_end: 40000,
        }
    }
}

// ---------------------------------------------------------------------------
// SDP helpers
// ---------------------------------------------------------------------------

/// Parsed connection/media information from an SDP body.
#[derive(Debug, Clone)]
pub struct SdpMediaInfo {
    /// Connection address.
    pub address: String,
    /// Audio media port.
    pub port: u16,
    /// Payload type numbers offered.
    pub payload_types: Vec<u16>,
    /// Codec names (from a=rtpmap lines), parallel to `payload_types` where available.
    pub codec_names: Vec<String>,
}

/// Minimally parse an SDP body to extract the connection address and first
/// audio m-line. This is intentionally simple – full SDP parsing lives in
/// `sip-core`.
pub fn parse_sdp_media(sdp: &str) -> Option<SdpMediaInfo> {
    let mut address: Option<String> = None;
    let mut port: Option<u16> = None;
    let mut payload_types: Vec<u16> = Vec::new();
    let mut codec_names: Vec<String> = Vec::new();

    for line in sdp.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("c=IN IP4 ") {
            // Take the first token (address); ignore TTL/count.
            if let Some(addr) = rest.split_whitespace().next() {
                address = Some(addr.to_string());
            }
        } else if line.starts_with("m=audio ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                port = parts[1].parse().ok();
                for pt in &parts[3..] {
                    if let Ok(n) = pt.parse::<u16>() {
                        payload_types.push(n);
                    }
                }
            }
        } else if let Some(rest) = line.strip_prefix("a=rtpmap:") {
            // e.g. "0 PCMU/8000"
            let tokens: Vec<&str> = rest.split_whitespace().collect();
            if tokens.len() >= 2 {
                if let Some(name) = tokens[1].split('/').next() {
                    codec_names.push(name.to_string());
                }
            }
        }
    }

    match (address, port) {
        (Some(a), Some(p)) => Some(SdpMediaInfo {
            address: a,
            port: p,
            payload_types,
            codec_names,
        }),
        _ => None,
    }
}

/// Generate a minimal SDP offer/answer with the B2BUA's own media address.
///
/// `local_ip`   – the B2BUA's IP that will appear in c= and o= lines.
/// `local_port` – the RTP port the B2BUA will listen on for this leg.
/// `codecs`     – codec list to advertise (payload type, name, clock rate).
pub fn generate_b2bua_sdp(
    local_ip: &str,
    local_port: u16,
    codecs: &[(u16, &str, u32)],
) -> String {
    let mut sdp = String::with_capacity(512);
    sdp.push_str("v=0\r\n");
    sdp.push_str(&format!(
        "o=- {} 0 IN IP4 {}\r\n",
        local_port, local_ip
    ));
    sdp.push_str("s=B2BUA Session\r\n");
    sdp.push_str(&format!("c=IN IP4 {}\r\n", local_ip));
    sdp.push_str("t=0 0\r\n");

    // m= line with all payload types
    let pts: Vec<String> = codecs.iter().map(|(pt, _, _)| pt.to_string()).collect();
    sdp.push_str(&format!("m=audio {} RTP/AVP {}\r\n", local_port, pts.join(" ")));

    // a=rtpmap lines
    for (pt, name, clock) in codecs {
        sdp.push_str(&format!("a=rtpmap:{} {}/{}\r\n", pt, name, clock));
    }
    sdp.push_str("a=sendrecv\r\n");

    sdp
}

/// Pick the common codecs between an offered SDP and a set of supported codecs.
///
/// Returns the intersection ordered by the offerer's preference.
pub fn negotiate_codecs(
    offered: &SdpMediaInfo,
    supported: &[(u16, &str, u32)],
) -> Vec<(u16, String, u32)> {
    let supported_names: Vec<&str> = supported.iter().map(|(_, n, _)| *n).collect();
    let mut result = Vec::new();

    // First pass: match by codec name in rtpmap
    for name in &offered.codec_names {
        let upper = name.to_uppercase();
        for &(pt, sname, clock) in supported {
            if sname.to_uppercase() == upper && !result.iter().any(|(_, n, _): &(u16, String, u32)| n.to_uppercase() == upper) {
                result.push((pt, sname.to_string(), clock));
            }
        }
    }

    // Second pass: match by well-known payload type number (0=PCMU, 8=PCMA)
    for pt in &offered.payload_types {
        let matched = match pt {
            0 if supported_names.contains(&"PCMU") => Some((0u16, "PCMU".to_string(), 8000u32)),
            8 if supported_names.contains(&"PCMA") => Some((8, "PCMA".to_string(), 8000)),
            _ => None,
        };
        if let Some(m) = matched {
            if !result.iter().any(|(_, n, _)| *n == m.1) {
                result.push(m);
            }
        }
    }

    result
}

/// The default set of codecs the B2BUA supports.
pub const DEFAULT_CODECS: &[(u16, &str, u32)] = &[
    (0, "PCMU", 8000),
    (8, "PCMA", 8000),
];

// ---------------------------------------------------------------------------
// B2BUA Engine
// ---------------------------------------------------------------------------

/// B2BUA session coordinator.
///
/// Manages paired inbound/outbound call legs, media bridging, and header
/// manipulation for all active B2BUA sessions.
pub struct B2BUACoordinator {
    routing_engine: Arc<dyn crate::routing::RoutingEngine>,
    policy_engine: Arc<dyn crate::policy::PolicyEngine>,
    sessions: Arc<RwLock<HashMap<IntermediarySessionId, B2buaSession>>>,
    config: B2buaConfig,
    /// Next media port to allocate (simple bump allocator).
    next_media_port: Arc<RwLock<u16>>,
}

impl B2BUACoordinator {
    /// Create a new B2BUA coordinator.
    pub fn new(
        routing_engine: Arc<dyn crate::routing::RoutingEngine>,
        policy_engine: Arc<dyn crate::policy::PolicyEngine>,
        config: B2buaConfig,
    ) -> Self {
        let media_start = config.media_port_start;
        Self {
            routing_engine,
            policy_engine,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            config,
            next_media_port: Arc::new(RwLock::new(media_start)),
        }
    }

    /// Create a coordinator with default config.
    pub fn with_defaults(
        routing_engine: Arc<dyn crate::routing::RoutingEngine>,
        policy_engine: Arc<dyn crate::policy::PolicyEngine>,
    ) -> Self {
        Self::new(routing_engine, policy_engine, B2buaConfig::default())
    }

    // ----- port allocator --------------------------------------------------

    /// Allocate the next even RTP port pair from the configured range.
    async fn allocate_media_port(&self) -> Result<u16> {
        let mut port = self.next_media_port.write().await;
        // Ensure even port (RTP convention).
        if *port % 2 != 0 {
            *port += 1;
        }
        let allocated = *port;
        if allocated >= self.config.media_port_end {
            return Err(crate::common::errors::IntermediaryError::BridgeError(
                "Media port range exhausted".to_string(),
            ));
        }
        *port = allocated + 2; // reserve pair (RTP + RTCP)
        Ok(allocated)
    }

    // ----- header manipulation helpers -------------------------------------

    /// Apply configured header manipulation rules.
    ///
    /// Returns the filtered/rewritten header list that should be used on the
    /// outbound leg.
    pub fn manipulate_headers(
        &self,
        inbound_headers: &[(String, String)],
    ) -> Vec<(String, String)> {
        let rules = &self.config.header_rules;
        let strip_lower: Vec<String> = rules
            .strip_headers
            .iter()
            .map(|h| h.to_lowercase())
            .collect();

        let mut out: Vec<(String, String)> = inbound_headers
            .iter()
            .filter(|(name, _)| !strip_lower.contains(&name.to_lowercase()))
            .cloned()
            .collect();

        // Apply rewrites
        for (name, new_val) in &rules.rewrite_headers {
            let lower = name.to_lowercase();
            for entry in &mut out {
                if entry.0.to_lowercase() == lower {
                    entry.1 = new_val.clone();
                }
            }
        }

        // Append additional headers
        for (name, value) in &rules.add_headers {
            out.push((name.clone(), value.clone()));
        }

        out
    }

    // ----- SDP manipulation ------------------------------------------------

    /// Given the caller's SDP offer, produce a new SDP for the outbound leg
    /// that advertises the B2BUA's own media address and negotiated codecs.
    pub async fn rewrite_sdp_for_outbound(
        &self,
        inbound_sdp: &str,
    ) -> Result<(String, u16)> {
        let offered = parse_sdp_media(inbound_sdp).ok_or_else(|| {
            crate::common::errors::IntermediaryError::BridgeError(
                "Failed to parse inbound SDP".to_string(),
            )
        })?;

        let negotiated = negotiate_codecs(&offered, DEFAULT_CODECS);
        if negotiated.is_empty() {
            return Err(crate::common::errors::IntermediaryError::BridgeError(
                "No common codecs between caller and B2BUA".to_string(),
            ));
        }

        let local_port = self.allocate_media_port().await?;
        let codecs: Vec<(u16, &str, u32)> = negotiated
            .iter()
            .map(|(pt, name, clock)| (*pt, name.as_str(), *clock))
            .collect();

        let sdp = generate_b2bua_sdp(&self.config.local_ip, local_port, &codecs);
        Ok((sdp, local_port))
    }

    /// After receiving the callee's SDP answer on leg B, produce the SDP
    /// answer to send back on leg A.  The B2BUA substitutes its own media
    /// address so that RTP flows through it.
    pub async fn rewrite_sdp_for_inbound_answer(
        &self,
        outbound_answer_sdp: &str,
    ) -> Result<(String, u16)> {
        let answered = parse_sdp_media(outbound_answer_sdp).ok_or_else(|| {
            crate::common::errors::IntermediaryError::BridgeError(
                "Failed to parse outbound SDP answer".to_string(),
            )
        })?;

        let local_port = self.allocate_media_port().await?;

        // Use the codecs from the answer (already negotiated by leg B).
        let codecs: Vec<(u16, &str, u32)> = answered
            .codec_names
            .iter()
            .zip(answered.payload_types.iter())
            .map(|(name, pt)| (*pt, name.as_str(), 8000u32))
            .collect();

        let codecs_final = if codecs.is_empty() {
            // Fallback to well-known types from the answer
            answered
                .payload_types
                .iter()
                .filter_map(|pt| match pt {
                    0 => Some((0u16, "PCMU", 8000u32)),
                    8 => Some((8u16, "PCMA", 8000u32)),
                    _ => None,
                })
                .collect()
        } else {
            codecs
        };

        let sdp = generate_b2bua_sdp(&self.config.local_ip, local_port, &codecs_final);
        Ok((sdp, local_port))
    }

    // ----- call lifecycle --------------------------------------------------

    /// Handle an incoming INVITE on the inbound leg.
    ///
    /// 1. Evaluate policies (reject if denied).
    /// 2. Determine routing target via the routing engine.
    /// 3. Rewrite SDP for the outbound leg.
    /// 4. Return the pair-id and the rewritten SDP + routing target so the
    ///    caller can create the outbound call.
    pub async fn handle_incoming_invite(
        &self,
        from: &str,
        to: &str,
        inbound_sdp: Option<&str>,
        inbound_headers: &[(String, String)],
    ) -> Result<IncomingInviteResult> {
        // 1. Policy check
        let policies = self
            .policy_engine
            .evaluate(from, to, "INVITE", inbound_headers)
            .await?;

        for policy in &policies {
            if let PolicyAction::Reject { code, reason } = policy {
                return Err(crate::common::errors::IntermediaryError::PolicyViolation(
                    format!("{}: {}", code, reason),
                ));
            }
        }

        // 2. Routing
        let decision = self
            .routing_engine
            .route(from, to, "INVITE", inbound_headers)
            .await?;

        if decision.targets.is_empty() {
            return Err(crate::common::errors::IntermediaryError::RoutingError(
                "No routing targets available".to_string(),
            ));
        }

        // 3. SDP manipulation
        let (outbound_sdp, outbound_media_port) = if let Some(sdp) = inbound_sdp {
            let (s, p) = self.rewrite_sdp_for_outbound(sdp).await?;
            (Some(s), Some(p))
        } else {
            (None, None)
        };

        // 4. Header manipulation
        let outbound_headers = self.manipulate_headers(inbound_headers);

        let pair_id = IntermediarySessionId::new();

        Ok(IncomingInviteResult {
            pair_id,
            routing_target: decision.targets[0].clone(),
            outbound_sdp,
            outbound_media_port,
            outbound_headers,
            inbound_sdp: inbound_sdp.map(String::from),
        })
    }

    /// Register a fully established B2BUA session (both legs up, bridge ready).
    pub async fn register_session(&self, session: B2buaSession) {
        let id = session.id.clone();
        self.sessions.write().await.insert(id, session);
    }

    /// Bridge media between two legs using the session-core `CallBridge`.
    pub async fn bridge_media(
        &self,
        pair_id: &IntermediarySessionId,
    ) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        let session = sessions.get_mut(pair_id).ok_or_else(|| {
            crate::common::errors::IntermediaryError::SessionNotFound(pair_id.0.clone())
        })?;

        if session.state != B2buaSessionState::SettingUp {
            return Err(crate::common::errors::IntermediaryError::BridgeError(
                format!("Cannot bridge session in state {:?}", session.state),
            ));
        }

        // Use the coordinator from the inbound call to create a bridge.
        let bridge_id = session
            .inbound_call
            .coordinator()
            .bridge_sessions(session.inbound_call.id(), session.outbound_call.id())
            .await
            .map_err(|e| {
                crate::common::errors::IntermediaryError::BridgeError(format!(
                    "Failed to bridge sessions: {}",
                    e
                ))
            })?;

        tracing::info!(
            pair_id = %pair_id.0,
            bridge = %bridge_id,
            "Media bridged between inbound and outbound legs"
        );

        session.state = B2buaSessionState::Active;
        Ok(())
    }

    /// Handle a BYE received on one leg: forward it to the other leg and
    /// tear down the session.
    pub async fn handle_bye(
        &self,
        pair_id: &IntermediarySessionId,
    ) -> Result<()> {
        let session = {
            let mut sessions = self.sessions.write().await;
            let session = sessions.get_mut(pair_id).ok_or_else(|| {
                crate::common::errors::IntermediaryError::SessionNotFound(pair_id.0.clone())
            })?;

            if session.state == B2buaSessionState::Terminated
                || session.state == B2buaSessionState::Terminating
            {
                return Ok(()); // already being torn down
            }

            session.state = B2buaSessionState::Terminating;
            // We need the pair_id to remove later; clone what we need.
            pair_id.clone()
        };

        // Terminate outside the write lock to avoid deadlocks with the
        // coordinator's own locks.
        self.terminate_session_internal(&session).await
    }

    /// Handle a re-INVITE on one leg by rewriting SDP and forwarding.
    ///
    /// Returns the rewritten SDP for the other leg.
    pub async fn handle_reinvite(
        &self,
        pair_id: &IntermediarySessionId,
        new_sdp: &str,
        _from_inbound: bool,
    ) -> Result<String> {
        let sessions = self.sessions.read().await;
        let _session = sessions.get(pair_id).ok_or_else(|| {
            crate::common::errors::IntermediaryError::SessionNotFound(pair_id.0.clone())
        })?;

        // Rewrite the SDP with B2BUA's own address regardless of direction.
        let (rewritten, _port) = self.rewrite_sdp_for_outbound(new_sdp).await?;

        tracing::info!(
            pair_id = %pair_id.0,
            "Re-INVITE SDP rewritten for forwarding"
        );

        Ok(rewritten)
    }

    /// Terminate a B2BUA session, hanging up both legs.
    pub async fn terminate_session(
        &self,
        pair_id: &IntermediarySessionId,
    ) -> Result<()> {
        {
            let mut sessions = self.sessions.write().await;
            if let Some(session) = sessions.get_mut(pair_id) {
                session.state = B2buaSessionState::Terminating;
            } else {
                return Err(crate::common::errors::IntermediaryError::SessionNotFound(
                    pair_id.0.clone(),
                ));
            }
        }

        self.terminate_session_internal(pair_id).await
    }

    /// Internal helper – terminates both legs and removes session from the map.
    async fn terminate_session_internal(
        &self,
        pair_id: &IntermediarySessionId,
    ) -> Result<()> {
        // Remove from the map so no other operations can touch it.
        let session = self.sessions.write().await.remove(pair_id);

        if let Some(session) = session {
            // Disconnect bridge first (if any).
            if let Some(bridge) = &session.bridge {
                if let Err(e) = bridge.disconnect().await {
                    tracing::warn!(
                        pair_id = %pair_id.0,
                        error = %e,
                        "Error disconnecting bridge during teardown"
                    );
                }
            }

            tracing::info!(
                pair_id = %pair_id.0,
                "B2BUA session terminated"
            );
        }

        Ok(())
    }

    /// Get the current state of a B2BUA session.
    pub async fn session_state(
        &self,
        pair_id: &IntermediarySessionId,
    ) -> Result<B2buaSessionState> {
        let sessions = self.sessions.read().await;
        sessions
            .get(pair_id)
            .map(|s| s.state.clone())
            .ok_or_else(|| {
                crate::common::errors::IntermediaryError::SessionNotFound(pair_id.0.clone())
            })
    }

    /// Return the number of active B2BUA sessions.
    pub async fn active_session_count(&self) -> usize {
        self.sessions
            .read()
            .await
            .values()
            .filter(|s| s.state == B2buaSessionState::Active)
            .count()
    }

    /// List all session pair IDs.
    pub async fn list_sessions(&self) -> Vec<IntermediarySessionId> {
        self.sessions.read().await.keys().cloned().collect()
    }
}

/// Result returned by `handle_incoming_invite` with everything needed to
/// create the outbound call leg.
#[derive(Debug)]
pub struct IncomingInviteResult {
    /// The unique pair id for this B2BUA session.
    pub pair_id: IntermediarySessionId,
    /// The routing target URI for the outbound INVITE.
    pub routing_target: String,
    /// Rewritten SDP for the outbound leg (B2BUA's address).
    pub outbound_sdp: Option<String>,
    /// Media port allocated for the outbound leg.
    pub outbound_media_port: Option<u16>,
    /// Headers for the outbound INVITE after manipulation.
    pub outbound_headers: Vec<(String, String)>,
    /// Original inbound SDP (kept for reference / re-INVITE).
    pub inbound_sdp: Option<String>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sdp_media_basic() {
        let sdp = "v=0\r\n\
                    o=- 0 0 IN IP4 192.168.1.10\r\n\
                    s=-\r\n\
                    c=IN IP4 192.168.1.10\r\n\
                    t=0 0\r\n\
                    m=audio 5004 RTP/AVP 0 8\r\n\
                    a=rtpmap:0 PCMU/8000\r\n\
                    a=rtpmap:8 PCMA/8000\r\n";

        let info = parse_sdp_media(sdp);
        assert!(info.is_some());
        let info = info.unwrap();
        assert_eq!(info.address, "192.168.1.10");
        assert_eq!(info.port, 5004);
        assert_eq!(info.payload_types, vec![0, 8]);
        assert_eq!(info.codec_names, vec!["PCMU", "PCMA"]);
    }

    #[test]
    fn test_generate_b2bua_sdp() {
        let sdp = generate_b2bua_sdp("10.0.0.1", 30000, &[(0, "PCMU", 8000), (8, "PCMA", 8000)]);
        assert!(sdp.contains("c=IN IP4 10.0.0.1"));
        assert!(sdp.contains("m=audio 30000 RTP/AVP 0 8"));
        assert!(sdp.contains("a=rtpmap:0 PCMU/8000"));
        assert!(sdp.contains("a=sendrecv"));
    }

    #[test]
    fn test_negotiate_codecs_common() {
        let offered = SdpMediaInfo {
            address: "10.0.0.1".to_string(),
            port: 5000,
            payload_types: vec![0, 8, 101],
            codec_names: vec!["PCMU".to_string(), "PCMA".to_string(), "telephone-event".to_string()],
        };

        let result = negotiate_codecs(&offered, DEFAULT_CODECS);
        assert!(!result.is_empty());
        assert_eq!(result[0].1, "PCMU");
    }

    #[test]
    fn test_negotiate_codecs_no_match() {
        let offered = SdpMediaInfo {
            address: "10.0.0.1".to_string(),
            port: 5000,
            payload_types: vec![111],
            codec_names: vec!["opus".to_string()],
        };

        let result = negotiate_codecs(&offered, DEFAULT_CODECS);
        assert!(result.is_empty());
    }

    #[test]
    fn test_header_manipulation_defaults() {
        let rules = HeaderManipulation::default();
        assert!(rules.strip_headers.contains(&"Via".to_string()));
        assert!(rules.strip_headers.contains(&"Record-Route".to_string()));
    }

    #[test]
    fn test_manipulate_headers() {
        let routing = Arc::new(crate::routing::BasicRoutingEngine::new());
        let policy = Arc::new(crate::policy::BasicPolicyEngine::new());
        let coordinator = B2BUACoordinator::with_defaults(routing, policy);

        let inbound = vec![
            ("Via".to_string(), "SIP/2.0/UDP caller:5060".to_string()),
            ("From".to_string(), "sip:alice@example.com".to_string()),
            ("Record-Route".to_string(), "<sip:proxy@example.com>".to_string()),
            ("Max-Forwards".to_string(), "70".to_string()),
        ];

        let out = coordinator.manipulate_headers(&inbound);
        // Via and Record-Route should be stripped
        assert!(!out.iter().any(|(n, _)| n == "Via"));
        assert!(!out.iter().any(|(n, _)| n == "Record-Route"));
        // From and Max-Forwards should remain
        assert!(out.iter().any(|(n, _)| n == "From"));
        assert!(out.iter().any(|(n, _)| n == "Max-Forwards"));
    }

    #[tokio::test]
    async fn test_allocate_media_port() {
        let routing = Arc::new(crate::routing::BasicRoutingEngine::new());
        let policy = Arc::new(crate::policy::BasicPolicyEngine::new());
        let config = B2buaConfig {
            media_port_start: 30000,
            media_port_end: 30010,
            ..Default::default()
        };
        let coordinator = B2BUACoordinator::new(routing, policy, config);

        let p1 = coordinator.allocate_media_port().await.unwrap();
        assert_eq!(p1, 30000);
        let p2 = coordinator.allocate_media_port().await.unwrap();
        assert_eq!(p2, 30002);
    }

    #[tokio::test]
    async fn test_allocate_media_port_exhaustion() {
        let routing = Arc::new(crate::routing::BasicRoutingEngine::new());
        let policy = Arc::new(crate::policy::BasicPolicyEngine::new());
        let config = B2buaConfig {
            media_port_start: 30000,
            media_port_end: 30002,
            ..Default::default()
        };
        let coordinator = B2BUACoordinator::new(routing, policy, config);

        let _ = coordinator.allocate_media_port().await.unwrap();
        let result = coordinator.allocate_media_port().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_rewrite_sdp_for_outbound() {
        let routing = Arc::new(crate::routing::BasicRoutingEngine::new());
        let policy = Arc::new(crate::policy::BasicPolicyEngine::new());
        let config = B2buaConfig {
            local_ip: "10.0.0.1".to_string(),
            media_port_start: 30000,
            media_port_end: 30100,
            ..Default::default()
        };
        let coordinator = B2BUACoordinator::new(routing, policy, config);

        let inbound_sdp = "v=0\r\n\
                           o=- 0 0 IN IP4 192.168.1.10\r\n\
                           c=IN IP4 192.168.1.10\r\n\
                           t=0 0\r\n\
                           m=audio 5004 RTP/AVP 0 8\r\n\
                           a=rtpmap:0 PCMU/8000\r\n";

        let (sdp, port) = coordinator.rewrite_sdp_for_outbound(inbound_sdp).await.unwrap();
        assert!(sdp.contains("c=IN IP4 10.0.0.1"));
        assert!(sdp.contains(&format!("m=audio {}", port)));
        assert!(!sdp.contains("192.168.1.10"));
    }

    #[tokio::test]
    async fn test_handle_incoming_invite_policy_reject() {
        use crate::common::errors::IntermediaryError;

        struct RejectPolicy;
        #[async_trait::async_trait]
        impl crate::policy::PolicyEngine for RejectPolicy {
            async fn evaluate(&self, _: &str, _: &str, _: &str, _: &[(String, String)]) -> Result<Vec<PolicyAction>> {
                Ok(vec![PolicyAction::Reject { code: 403, reason: "Forbidden".to_string() }])
            }
            async fn is_policy_enabled(&self, _: &str) -> bool { false }
        }

        let routing = Arc::new(crate::routing::BasicRoutingEngine::new());
        let policy: Arc<dyn crate::policy::PolicyEngine> = Arc::new(RejectPolicy);
        let coordinator = B2BUACoordinator::with_defaults(routing, policy);

        let result = coordinator
            .handle_incoming_invite("alice@a.com", "bob@b.com", None, &[])
            .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            IntermediaryError::PolicyViolation(msg) => {
                assert!(msg.contains("403"));
            }
            other => panic!("Expected PolicyViolation, got {:?}", other),
        }
    }
}
