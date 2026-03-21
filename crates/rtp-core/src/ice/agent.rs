//! ICE Agent state machine per RFC 8445.
//!
//! The [`IceAgent`] orchestrates candidate gathering, pair formation,
//! connectivity checks, and nomination to establish a media path through NAT.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{debug, info, trace, warn};

/// Interval between ICE consent freshness checks per RFC 7675.
const CONSENT_CHECK_INTERVAL: Duration = Duration::from_secs(15);

/// Timeout after which consent is considered expired per RFC 7675.
const CONSENT_EXPIRY_TIMEOUT: Duration = Duration::from_secs(30);

use crate::Error;
use crate::stun::message::{StunMessage, StunAttribute, BINDING_RESPONSE, BINDING_REQUEST};
use crate::turn::TurnServerConfig;
use crate::turn::client::TurnClient;
use super::checklist;
use super::gather;
use super::types::{
    CandidateType, CandidatePairState, ComponentId, IceCandidate,
    IceCandidatePair, IceConnectionState, IceCredentials, IceRole,
};

/// The ICE agent manages the full ICE process: gathering, checking, and nomination.
pub struct IceAgent {
    /// This agent's role (controlling or controlled).
    role: IceRole,
    /// Local ICE credentials.
    local_credentials: IceCredentials,
    /// Remote ICE credentials (set after signaling exchange).
    remote_credentials: Option<IceCredentials>,
    /// Gathered local candidates.
    local_candidates: Vec<IceCandidate>,
    /// Remote candidates received via signaling.
    remote_candidates: Vec<IceCandidate>,
    /// The connectivity check list.
    checklist: Vec<IceCandidatePair>,
    /// Current connection state.
    state: IceConnectionState,
    /// The selected (nominated) candidate pair.
    selected_pair: Option<IceCandidatePair>,
    /// Tie-breaker value for role conflict resolution.
    tie_breaker: u64,
    /// Mapping from transaction ID to checklist index for in-flight checks.
    pending_checks: std::collections::HashMap<[u8; 12], usize>,
    /// Component ID this agent manages.
    component: ComponentId,
    /// Active TURN client handles for relay allocations.
    ///
    /// Stored so the caller can refresh, create permissions, or deallocate
    /// when the session ends.
    turn_clients: Vec<TurnClient>,

    // --- Trickle ICE (RFC 8838/8840) ---

    /// Whether trickle ICE is enabled for this agent.
    trickle_enabled: bool,
    /// Whether an end-of-candidates indication has been received/sent.
    end_of_candidates: bool,

    // --- ICE consent freshness (RFC 7675) ---

    /// Timestamp of the last successful consent response.
    last_consent_response: Option<Instant>,
    /// Number of consecutive consent check failures.
    consent_failures: u32,
    /// Transaction ID of the outstanding consent check, if any.
    pending_consent_txn: Option<[u8; 12]>,
}

impl IceAgent {
    /// Create a new ICE agent with the specified role.
    ///
    /// Generates random ICE credentials (4-char ufrag, 22-char password)
    /// and a random tie-breaker value.
    pub fn new(role: IceRole) -> Self {
        let credentials = IceCredentials::generate();
        let tie_breaker = rand::Rng::r#gen(&mut rand::thread_rng());

        debug!(
            role = %role,
            ufrag = %credentials.ufrag,
            "created ICE agent"
        );

        Self {
            role,
            local_credentials: credentials,
            remote_credentials: None,
            local_candidates: Vec::new(),
            remote_candidates: Vec::new(),
            checklist: Vec::new(),
            state: IceConnectionState::New,
            selected_pair: None,
            tie_breaker,
            pending_checks: std::collections::HashMap::new(),
            component: ComponentId::Rtp,
            turn_clients: Vec::new(),
            trickle_enabled: false,
            end_of_candidates: false,
            last_consent_response: None,
            consent_failures: u32::default(),
            pending_consent_txn: None,
        }
    }

    /// Create a new ICE agent for a specific component.
    pub fn with_component(role: IceRole, component: ComponentId) -> Self {
        let mut agent = Self::new(role);
        agent.component = component;
        agent
    }

    /// Get the agent's role.
    pub fn role(&self) -> IceRole {
        self.role
    }

    /// Get the local ICE credentials.
    pub fn local_credentials(&self) -> &IceCredentials {
        &self.local_credentials
    }

    /// Get the remote ICE credentials, if set.
    pub fn remote_credentials(&self) -> Option<&IceCredentials> {
        self.remote_credentials.as_ref()
    }

    /// Get the current connection state.
    pub fn state(&self) -> IceConnectionState {
        self.state
    }

    /// Get the selected (nominated) candidate pair.
    pub fn selected_pair(&self) -> Option<&IceCandidatePair> {
        self.selected_pair.as_ref()
    }

    /// Get all local candidates.
    pub fn local_candidates(&self) -> &[IceCandidate] {
        &self.local_candidates
    }

    /// Get the current checklist.
    pub fn checklist(&self) -> &[IceCandidatePair] {
        &self.checklist
    }

    /// Get the component this agent manages.
    pub fn component(&self) -> ComponentId {
        self.component
    }

    /// Set the remote ICE credentials received via signaling.
    pub fn set_remote_credentials(&mut self, ufrag: String, pwd: String) {
        debug!(
            remote_ufrag = %ufrag,
            "set remote ICE credentials"
        );
        self.remote_credentials = Some(IceCredentials { ufrag, pwd });
    }

    // ---------------------------------------------------------------
    // Trickle ICE (RFC 8838 / RFC 8840)
    //
    // Usage:
    // 1. Call agent.enable_trickle() before generating the SDP offer.
    // 2. gather_host_candidates_only() returns fast host candidates.
    // 3. Spawn background STUN/TURN gathering; as each candidate arrives
    //    call the session-layer trickle send helper.
    // 4. When gathering is done call agent.set_end_of_candidates().
    // 5. On the receiving side call agent.add_remote_candidate() for
    //    each trickled candidate, and agent.set_end_of_candidates()
    //    when the remote side signals end-of-candidates.
    // ---------------------------------------------------------------

    /// Enable trickle ICE mode (RFC 8838).
    ///
    /// When trickle is enabled the initial SDP offer/answer includes
    /// `a=ice-options:trickle` and only host candidates. Additional
    /// candidates are sent incrementally via SIP INFO.
    pub fn enable_trickle(&mut self) {
        self.trickle_enabled = true;
        debug!("trickle ICE enabled");
    }

    /// Returns whether trickle ICE is enabled.
    pub fn is_trickle_enabled(&self) -> bool {
        self.trickle_enabled
    }

    /// Mark that candidate gathering is complete (end-of-candidates).
    ///
    /// Per RFC 8838, once end-of-candidates is signalled no further
    /// candidates will be provided. If all checks are already done the
    /// agent can finalize.
    pub fn set_end_of_candidates(&mut self) {
        self.end_of_candidates = true;
        info!("end-of-candidates signalled");
        // If we are already checking we may be able to conclude
        self.update_state();
    }

    /// Returns whether end-of-candidates has been signalled.
    pub fn has_end_of_candidates(&self) -> bool {
        self.end_of_candidates
    }

    /// Gather only host candidates (synchronous, fast).
    ///
    /// Used for trickle ICE to return an SDP offer immediately with
    /// host candidates only. STUN/TURN gathering is done separately
    /// in a background task.
    pub fn gather_host_candidates_only(
        &mut self,
        local_addr: SocketAddr,
    ) -> Vec<IceCandidate> {
        self.state = IceConnectionState::Gathering;

        let host_candidates = gather::gather_host_candidates(
            local_addr,
            self.component,
            &self.local_credentials.ufrag,
        );

        for candidate in &host_candidates {
            debug!(candidate = %candidate, "gathered host candidate (trickle)");
        }
        self.local_candidates.extend(host_candidates.clone());

        debug!(
            count = host_candidates.len(),
            "trickle: host candidate gathering complete"
        );

        host_candidates
    }

    /// Gather local candidates for the given local address and STUN servers.
    ///
    /// This collects host candidates from local interfaces and optionally
    /// server-reflexive candidates via STUN servers. Transitions the agent
    /// state to `Gathering` during the process.
    pub async fn gather_candidates(
        &mut self,
        local_addr: SocketAddr,
        stun_servers: &[SocketAddr],
    ) -> Result<Vec<IceCandidate>, Error> {
        self.gather_candidates_with_turn(local_addr, stun_servers, &[]).await
    }

    /// Gather local candidates including TURN relay candidates.
    ///
    /// Collects host candidates, server-reflexive candidates (via STUN),
    /// and relay candidates (via TURN). The RTP session socket is reused
    /// for both STUN and TURN transactions to avoid binding extra ports.
    ///
    /// TURN client handles are stored in `self.turn_clients` so allocations
    /// can be refreshed and torn down later.
    pub async fn gather_candidates_with_turn(
        &mut self,
        local_addr: SocketAddr,
        stun_servers: &[SocketAddr],
        turn_configs: &[TurnServerConfig],
    ) -> Result<Vec<IceCandidate>, Error> {
        self.state = IceConnectionState::Gathering;

        debug!(
            local = %local_addr,
            stun_servers = stun_servers.len(),
            turn_servers = turn_configs.len(),
            "gathering ICE candidates"
        );

        // Gather host candidates
        let host_candidates = gather::gather_host_candidates(
            local_addr,
            self.component,
            &self.local_credentials.ufrag,
        );

        for candidate in &host_candidates {
            debug!(candidate = %candidate, "gathered host candidate");
        }
        self.local_candidates.extend(host_candidates);

        // Bind a socket shared by STUN and TURN gathering.
        // Reusing a single socket avoids extra port allocations and
        // matches the eventual RTP session socket.
        let needs_socket = !stun_servers.is_empty() || !turn_configs.is_empty();
        let socket = if needs_socket {
            let sock = tokio::net::UdpSocket::bind(local_addr).await.map_err(|e| {
                Error::IceError(format!("failed to bind UDP socket for ICE gathering: {e}"))
            })?;
            Some(Arc::new(sock))
        } else {
            None
        };

        // Gather server-reflexive candidates if STUN servers are provided
        if !stun_servers.is_empty() {
            if let Some(ref sock) = socket {
                let srflx_candidates = gather::gather_srflx_candidates(
                    sock,
                    stun_servers,
                    self.component,
                    &self.local_credentials.ufrag,
                )
                .await;

                for candidate in &srflx_candidates {
                    debug!(candidate = %candidate, "gathered srflx candidate");
                }
                self.local_candidates.extend(srflx_candidates);
            }
        }

        // Gather relay candidates via TURN servers
        if !turn_configs.is_empty() {
            if let Some(ref sock) = socket {
                let (relay_candidates, clients) = gather::gather_relay_candidates(
                    turn_configs,
                    sock,
                    self.component,
                    &self.local_credentials.ufrag,
                )
                .await;

                for candidate in &relay_candidates {
                    debug!(candidate = %candidate, "gathered relay candidate");
                }
                self.local_candidates.extend(relay_candidates);
                self.turn_clients.extend(clients);
            }
        }

        info!(
            count = self.local_candidates.len(),
            "candidate gathering complete"
        );

        Ok(self.local_candidates.clone())
    }

    /// Get a reference to the active TURN clients.
    pub fn turn_clients(&self) -> &[TurnClient] {
        &self.turn_clients
    }

    /// Take ownership of the TURN clients (e.g. for shutdown / deallocation).
    pub fn take_turn_clients(&mut self) -> Vec<TurnClient> {
        std::mem::take(&mut self.turn_clients)
    }

    /// Add a remote candidate received via signaling.
    ///
    /// If remote credentials have been set, this also forms new candidate
    /// pairs with the new remote candidate.
    pub fn add_remote_candidate(&mut self, candidate: IceCandidate) {
        debug!(
            candidate = %candidate,
            "adding remote candidate"
        );

        self.remote_candidates.push(candidate);

        // Re-form pairs if we have both local and remote candidates
        if !self.local_candidates.is_empty() {
            self.rebuild_checklist();
        }
    }

    /// Add multiple remote candidates at once.
    pub fn add_remote_candidates(&mut self, candidates: Vec<IceCandidate>) {
        for candidate in candidates {
            self.remote_candidates.push(candidate);
        }

        if !self.local_candidates.is_empty() {
            self.rebuild_checklist();
        }
    }

    /// Start connectivity checks.
    ///
    /// Initializes the checklist (unfreezes initial pairs) and transitions
    /// the state to `Checking`.
    pub fn start_checks(&mut self) -> Result<(), Error> {
        if self.remote_credentials.is_none() {
            return Err(Error::IceError(
                "cannot start checks: remote credentials not set".into(),
            ));
        }

        if self.checklist.is_empty() {
            return Err(Error::IceError(
                "cannot start checks: checklist is empty".into(),
            ));
        }

        self.state = IceConnectionState::Checking;
        checklist::initialize_checklist(&mut self.checklist);

        info!(
            pairs = self.checklist.len(),
            "started ICE connectivity checks"
        );

        Ok(())
    }

    /// Perform a connectivity check for the given pair index.
    ///
    /// Returns the encoded STUN request bytes and the remote address to send to.
    /// The pair state is set to `InProgress`.
    pub fn check_pair(&mut self, pair_idx: usize) -> Result<(Vec<u8>, SocketAddr), Error> {
        let remote_creds = self.remote_credentials.as_ref().ok_or_else(|| {
            Error::IceError("remote credentials not set".into())
        })?;

        if pair_idx >= self.checklist.len() {
            return Err(Error::IceError(format!(
                "pair index {} out of range (checklist has {} pairs)",
                pair_idx,
                self.checklist.len()
            )));
        }

        let pair = &self.checklist[pair_idx];
        let remote_addr = pair.remote.address;

        let nominate = self.role == IceRole::Controlling
            && pair.state == CandidatePairState::Succeeded;

        let (encoded, txn_id) = checklist::build_check_request(
            pair,
            &self.local_credentials,
            remote_creds,
            self.role,
            self.tie_breaker,
            nominate,
        );

        // Track the pending check
        self.pending_checks.insert(txn_id.0, pair_idx);

        // Mark pair as in-progress
        if let Some(p) = self.checklist.get_mut(pair_idx) {
            p.state = CandidatePairState::InProgress;
        }

        trace!(
            pair_idx = pair_idx,
            remote = %remote_addr,
            "sending connectivity check"
        );

        Ok((encoded, remote_addr))
    }

    /// Get the index of the next pair ready for checking.
    pub fn next_check(&self) -> Option<usize> {
        checklist::next_waiting_pair(&self.checklist)
    }

    /// Handle a STUN response received from the network.
    ///
    /// Processes Binding Responses and updates pair states accordingly.
    /// On success, may transition to Connected or Completed state.
    pub fn handle_stun_response(
        &mut self,
        response: &[u8],
        from: SocketAddr,
    ) -> Result<(), Error> {
        let msg = StunMessage::decode(response).map_err(|e| {
            Error::IceError(format!("failed to decode STUN response: {e}"))
        })?;

        // Look up the pending check by transaction ID
        let pair_idx = match self.pending_checks.remove(&msg.transaction_id.0) {
            Some(idx) => idx,
            None => {
                trace!(
                    txn = ?msg.transaction_id,
                    "received STUN response for unknown transaction"
                );
                return Ok(());
            }
        };

        if pair_idx >= self.checklist.len() {
            return Ok(());
        }

        if msg.msg_type == BINDING_RESPONSE {
            // Verify MESSAGE-INTEGRITY if remote credentials are available
            if let Some(ref remote_creds) = self.remote_credentials {
                if !msg.verify_integrity(response, remote_creds.pwd.as_bytes()) {
                    warn!(
                        pair_idx = pair_idx,
                        "STUN response failed integrity check"
                    );
                    if let Some(p) = self.checklist.get_mut(pair_idx) {
                        p.state = CandidatePairState::Failed;
                    }
                    self.update_state();
                    return Ok(());
                }
            }

            // Check succeeded
            if let Some(p) = self.checklist.get_mut(pair_idx) {
                p.state = CandidatePairState::Succeeded;

                debug!(
                    pair_idx = pair_idx,
                    local = %p.local.address,
                    remote = %p.remote.address,
                    "connectivity check succeeded"
                );

                // Check for peer-reflexive candidate from XOR-MAPPED-ADDRESS
                if let Some(mapped) = msg.mapped_address() {
                    if mapped != p.local.address {
                        trace!(
                            mapped = %mapped,
                            local = %p.local.address,
                            "discovered peer-reflexive address"
                        );
                        // Could create a peer-reflexive candidate here
                    }
                }

                // Handle nomination
                self.try_nominate(pair_idx);
            }

            self.update_state();
        } else {
            // Error response - mark pair as failed
            if let Some(p) = self.checklist.get_mut(pair_idx) {
                p.state = CandidatePairState::Failed;
                debug!(
                    pair_idx = pair_idx,
                    "connectivity check failed"
                );
            }
            self.update_state();
        }

        Ok(())
    }

    /// Handle an incoming STUN Binding Request (from the remote peer's check).
    ///
    /// `local_addr` is the local socket address on which the request was
    /// received.  It is used to find the correct local candidate when forming
    /// triggered-check pairs on multi-homed hosts.
    ///
    /// Returns the encoded STUN response bytes to send back, and the remote address.
    pub fn handle_stun_request(
        &mut self,
        request: &[u8],
        from: SocketAddr,
        local_addr: SocketAddr,
    ) -> Result<Option<Vec<u8>>, Error> {
        let msg = StunMessage::decode(request).map_err(|e| {
            Error::IceError(format!("failed to decode STUN request: {e}"))
        })?;

        if msg.msg_type != BINDING_REQUEST {
            return Ok(None);
        }

        // Verify USERNAME attribute
        let username = msg.attributes.iter().find_map(|a| {
            if let StunAttribute::Username(u) = a { Some(u.as_str()) } else { None }
        });

        let expected_username = format!(
            "{}:{}",
            self.local_credentials.ufrag,
            self.remote_credentials.as_ref().map(|c| c.ufrag.as_str()).unwrap_or(""),
        );

        if let Some(uname) = username {
            if uname != expected_username {
                trace!(
                    received = uname,
                    expected = %expected_username,
                    "USERNAME mismatch in incoming check"
                );
                return Ok(None);
            }
        }

        // Verify MESSAGE-INTEGRITY with local password
        if !msg.verify_integrity(request, self.local_credentials.pwd.as_bytes()) {
            warn!("incoming STUN request failed integrity check");
            return Ok(None);
        }

        // Build response
        let response = checklist::build_check_response(
            msg.transaction_id,
            &from,
            &self.local_credentials,
        );

        // Check if USE-CANDIDATE was present (remote is nominating)
        let use_candidate = msg.attributes.iter().any(|a| matches!(a, StunAttribute::UseCandidate));

        // Triggered check: find or create the pair for this check.
        // Use the actual local address the request arrived on so that
        // multi-homed hosts match the correct local candidate.
        let matched_local = self.local_candidates
            .iter()
            .find(|c| c.address == local_addr)
            .map(|c| c.address)
            .unwrap_or(local_addr);

        if let Some(pair_idx) = checklist::find_pair_by_addresses(
            &self.checklist,
            matched_local,
            from,
        ) {
            if let Some(p) = self.checklist.get_mut(pair_idx) {
                if p.state == CandidatePairState::Frozen || p.state == CandidatePairState::Waiting {
                    p.state = CandidatePairState::Succeeded;
                }
                if use_candidate {
                    p.nominated = true;
                    self.selected_pair = Some(p.clone());
                    debug!(
                        pair_idx = pair_idx,
                        "pair nominated by remote"
                    );
                }
            }
            self.update_state();
        }

        Ok(Some(response))
    }

    /// Attempt to nominate a pair (controlling agent only).
    fn try_nominate(&mut self, pair_idx: usize) {
        if self.role != IceRole::Controlling {
            return;
        }

        if let Some(p) = self.checklist.get_mut(pair_idx) {
            if p.state == CandidatePairState::Succeeded && !p.nominated {
                p.nominated = true;
                self.selected_pair = Some(p.clone());
                info!(
                    local = %p.local.address,
                    remote = %p.remote.address,
                    priority = p.priority,
                    "nominated candidate pair"
                );
            }
        }
    }

    /// Update the overall connection state based on checklist status.
    fn update_state(&mut self) {
        let has_nominated = self.selected_pair.is_some();
        let all_failed = !self.checklist.is_empty()
            && self.checklist.iter().all(|p| p.state == CandidatePairState::Failed);
        let any_succeeded = self.checklist.iter().any(|p| p.state == CandidatePairState::Succeeded);
        let all_done = self.checklist.iter().all(|p| {
            matches!(
                p.state,
                CandidatePairState::Succeeded | CandidatePairState::Failed
            )
        });

        let new_state = if has_nominated && all_done {
            IceConnectionState::Completed
        } else if has_nominated || any_succeeded {
            IceConnectionState::Connected
        } else if all_failed {
            IceConnectionState::Failed
        } else {
            self.state
        };

        if new_state != self.state {
            debug!(
                old = %self.state,
                new = %new_state,
                "ICE state transition"
            );
            self.state = new_state;
        }
    }

    /// Rebuild the checklist from current local and remote candidates.
    fn rebuild_checklist(&mut self) {
        self.checklist = checklist::form_candidate_pairs(
            &self.local_candidates,
            &self.remote_candidates,
            self.role,
        );
        checklist::sort_pairs(&mut self.checklist);
        checklist::prune_pairs(&mut self.checklist);

        debug!(
            pairs = self.checklist.len(),
            "rebuilt checklist"
        );
    }

    // ---------------------------------------------------------------
    // ICE consent freshness (RFC 7675)
    //
    // Usage from session-core media manager:
    // 1. Periodically (every 5s) call agent.needs_consent_check()
    // 2. If true: call agent.build_consent_check() and send via UDP
    // 3. On STUN response: call agent.handle_consent_response()
    // 4. Periodically call agent.check_consent_timeout()
    // 5. If timeout returns true: tear down media session
    // ---------------------------------------------------------------

    /// Returns true if a consent check is needed (15s since last response).
    ///
    /// Per RFC 7675, consent checks are authenticated STUN Binding Requests
    /// sent at regular intervals to verify that the remote peer still
    /// consents to receiving traffic.
    pub fn needs_consent_check(&self) -> bool {
        match (self.state, &self.selected_pair, &self.last_consent_response) {
            (
                IceConnectionState::Connected | IceConnectionState::Completed,
                Some(_),
                Some(last),
            ) => last.elapsed() > CONSENT_CHECK_INTERVAL,
            (
                IceConnectionState::Connected | IceConnectionState::Completed,
                Some(_),
                None,
            ) => true,
            _ => false,
        }
    }

    /// Build a consent check STUN Binding Request to the selected pair.
    ///
    /// Reuses `build_check_request` from `checklist.rs` with proper ICE
    /// credentials and role attributes. Returns the encoded STUN request
    /// and the destination socket address, or an error if preconditions
    /// are not met.
    pub fn build_consent_check(&mut self) -> Result<(Vec<u8>, SocketAddr), Error> {
        let pair = self.selected_pair.as_ref().ok_or_else(|| {
            Error::IceError("no selected pair for consent check".into())
        })?;

        let remote_creds = self.remote_credentials.as_ref().ok_or_else(|| {
            Error::IceError("remote credentials not set for consent check".into())
        })?;

        let remote_addr = pair.remote.address;

        // Consent checks never nominate; they only verify liveness.
        let (encoded, txn_id) = checklist::build_check_request(
            pair,
            &self.local_credentials,
            remote_creds,
            self.role,
            self.tie_breaker,
            false, // never nominate on consent checks
        );

        self.pending_consent_txn = Some(txn_id.0);

        trace!(
            remote = %remote_addr,
            "built consent freshness check"
        );

        Ok((encoded, remote_addr))
    }

    /// Handle a consent check response by verifying its transaction ID.
    ///
    /// If the transaction ID matches the outstanding consent check, resets
    /// the consent timer and failure counter.
    pub fn handle_consent_response(&mut self, transaction_id: &[u8; 12]) {
        let matches = self
            .pending_consent_txn
            .as_ref()
            .map_or(false, |pending| pending == transaction_id);

        if matches {
            self.last_consent_response = Some(Instant::now());
            self.consent_failures = 0;
            self.pending_consent_txn = None;
            trace!("consent freshness check succeeded");
        }
    }

    /// Returns true if consent has expired (no response within 30s).
    pub fn is_consent_expired(&self) -> bool {
        match &self.last_consent_response {
            Some(last) => last.elapsed() > CONSENT_EXPIRY_TIMEOUT,
            // No response yet -- cannot expire before the first check is sent
            None => false,
        }
    }

    /// Check whether consent has timed out and transition to Disconnected.
    ///
    /// Returns `true` if consent expired and the state was changed.
    pub fn check_consent_timeout(&mut self) -> bool {
        if self.is_consent_expired() {
            info!(
                failures = self.consent_failures,
                "consent expired, transitioning to Disconnected"
            );
            self.state = IceConnectionState::Disconnected;
            self.consent_failures += 1;
            true
        } else {
            false
        }
    }

    /// Get the number of consecutive consent failures.
    pub fn consent_failures(&self) -> u32 {
        self.consent_failures
    }

    /// Close the agent and release resources.
    pub fn close(&mut self) {
        self.state = IceConnectionState::Closed;
        self.pending_checks.clear();
        debug!("ICE agent closed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_new() {
        let agent = IceAgent::new(IceRole::Controlling);
        assert_eq!(agent.role(), IceRole::Controlling);
        assert_eq!(agent.state(), IceConnectionState::New);
        assert!(agent.selected_pair().is_none());
        assert_eq!(agent.local_credentials().ufrag.len(), 4);
        assert_eq!(agent.local_credentials().pwd.len(), 22);
    }

    #[test]
    fn test_agent_with_component() {
        let agent = IceAgent::with_component(IceRole::Controlled, ComponentId::Rtcp);
        assert_eq!(agent.component(), ComponentId::Rtcp);
        assert_eq!(agent.role(), IceRole::Controlled);
    }

    #[test]
    fn test_set_remote_credentials() {
        let mut agent = IceAgent::new(IceRole::Controlling);
        assert!(agent.remote_credentials().is_none());

        agent.set_remote_credentials("WXYZ".to_string(), "remote_pwd_22characters".to_string());
        let creds = agent.remote_credentials();
        assert!(creds.is_some());
        let creds = creds.unwrap_or_else(|| panic!("should have credentials"));
        assert_eq!(creds.ufrag, "WXYZ");
        assert_eq!(creds.pwd, "remote_pwd_22characters");
    }

    #[test]
    fn test_add_remote_candidate_forms_pairs() {
        let mut agent = IceAgent::new(IceRole::Controlling);

        // Add a local candidate manually
        agent.local_candidates.push(IceCandidate {
            foundation: "1".to_string(),
            component: ComponentId::Rtp,
            transport: "udp".to_string(),
            priority: 2130706431,
            address: "192.168.1.1:5000".parse().unwrap_or_else(|e| panic!("parse: {e}")),
            candidate_type: CandidateType::Host,
            related_address: None,
            ufrag: agent.local_credentials().ufrag.clone(),
        });

        // Add a remote candidate
        agent.add_remote_candidate(IceCandidate {
            foundation: "2".to_string(),
            component: ComponentId::Rtp,
            transport: "udp".to_string(),
            priority: 2130706431,
            address: "10.0.0.1:6000".parse().unwrap_or_else(|e| panic!("parse: {e}")),
            candidate_type: CandidateType::Host,
            related_address: None,
            ufrag: "remote".to_string(),
        });

        assert_eq!(agent.checklist().len(), 1);
    }

    #[test]
    fn test_start_checks_requires_credentials() {
        let mut agent = IceAgent::new(IceRole::Controlling);
        let result = agent.start_checks();
        assert!(result.is_err(), "should fail without remote credentials");
    }

    #[test]
    fn test_start_checks_requires_checklist() {
        let mut agent = IceAgent::new(IceRole::Controlling);
        agent.set_remote_credentials("test".to_string(), "password_22_characters!".to_string());
        let result = agent.start_checks();
        assert!(result.is_err(), "should fail with empty checklist");
    }

    #[test]
    fn test_start_checks_success() {
        let mut agent = IceAgent::new(IceRole::Controlling);
        agent.set_remote_credentials("test".to_string(), "password_22_characters!".to_string());

        // Add candidates
        agent.local_candidates.push(IceCandidate {
            foundation: "1".to_string(),
            component: ComponentId::Rtp,
            transport: "udp".to_string(),
            priority: 2130706431,
            address: "192.168.1.1:5000".parse().unwrap_or_else(|e| panic!("parse: {e}")),
            candidate_type: CandidateType::Host,
            related_address: None,
            ufrag: agent.local_credentials().ufrag.clone(),
        });
        agent.add_remote_candidate(IceCandidate {
            foundation: "2".to_string(),
            component: ComponentId::Rtp,
            transport: "udp".to_string(),
            priority: 2130706431,
            address: "10.0.0.1:6000".parse().unwrap_or_else(|e| panic!("parse: {e}")),
            candidate_type: CandidateType::Host,
            related_address: None,
            ufrag: "test".to_string(),
        });

        let result = agent.start_checks();
        assert!(result.is_ok());
        assert_eq!(agent.state(), IceConnectionState::Checking);

        // Should have at least one waiting pair
        assert!(agent.next_check().is_some());
    }

    #[test]
    fn test_check_pair_out_of_range() {
        let mut agent = IceAgent::new(IceRole::Controlling);
        agent.set_remote_credentials("test".to_string(), "password_22_characters!".to_string());
        let result = agent.check_pair(999);
        assert!(result.is_err());
    }

    #[test]
    fn test_close() {
        let mut agent = IceAgent::new(IceRole::Controlling);
        agent.close();
        assert_eq!(agent.state(), IceConnectionState::Closed);
    }

    // --- Consent freshness (RFC 7675) tests ---

    /// Helper: build an agent in Connected state with a selected pair and
    /// remote credentials set, ready for consent checks.
    fn make_connected_agent() -> IceAgent {
        let mut agent = IceAgent::new(IceRole::Controlling);
        agent.set_remote_credentials(
            "REMT".to_string(),
            "remote_password_22chars".to_string(),
        );

        let local = IceCandidate {
            foundation: "1".to_string(),
            component: ComponentId::Rtp,
            transport: "udp".to_string(),
            priority: 2130706431,
            address: "192.168.1.1:5000"
                .parse()
                .unwrap_or_else(|e| panic!("parse: {e}")),
            candidate_type: CandidateType::Host,
            related_address: None,
            ufrag: agent.local_credentials().ufrag.clone(),
        };
        let remote = IceCandidate {
            foundation: "2".to_string(),
            component: ComponentId::Rtp,
            transport: "udp".to_string(),
            priority: 2130706431,
            address: "10.0.0.1:6000"
                .parse()
                .unwrap_or_else(|e| panic!("parse: {e}")),
            candidate_type: CandidateType::Host,
            related_address: None,
            ufrag: "REMT".to_string(),
        };

        let pair = IceCandidatePair {
            local,
            remote,
            state: CandidatePairState::Succeeded,
            priority: 1000,
            nominated: true,
        };
        agent.selected_pair = Some(pair);
        agent.state = IceConnectionState::Connected;
        agent
    }

    #[test]
    fn test_needs_consent_check_false_before_15s() {
        let mut agent = make_connected_agent();
        agent.last_consent_response = Some(Instant::now());
        assert!(
            !agent.needs_consent_check(),
            "should not need consent check immediately after response"
        );
    }

    #[test]
    fn test_needs_consent_check_true_after_15s() {
        let mut agent = make_connected_agent();
        agent.last_consent_response = Some(Instant::now() - Duration::from_secs(16));
        assert!(
            agent.needs_consent_check(),
            "should need consent check after 15s"
        );
    }

    #[test]
    fn test_needs_consent_check_true_when_no_prior_response() {
        let agent = make_connected_agent();
        assert!(
            agent.needs_consent_check(),
            "should need consent check when no prior response exists"
        );
    }

    #[test]
    fn test_needs_consent_check_false_when_not_connected() {
        let mut agent = IceAgent::new(IceRole::Controlling);
        agent.state = IceConnectionState::Checking;
        assert!(
            !agent.needs_consent_check(),
            "should not need consent check when not connected"
        );
    }

    #[test]
    fn test_consent_response_resets_timer() {
        let mut agent = make_connected_agent();
        agent.last_consent_response = Some(Instant::now() - Duration::from_secs(20));
        agent.consent_failures = 3;

        let result = agent.build_consent_check();
        assert!(result.is_ok(), "build_consent_check should succeed");

        let txn = agent
            .pending_consent_txn
            .unwrap_or_else(|| panic!("should have pending consent txn"));

        agent.handle_consent_response(&txn);

        assert_eq!(agent.consent_failures, 0, "failures should be reset");
        assert!(
            agent.last_consent_response.is_some(),
            "last_consent_response should be set"
        );
        assert!(
            !agent.needs_consent_check(),
            "should not need check right after response"
        );
    }

    #[test]
    fn test_consent_timeout_after_30s() {
        let mut agent = make_connected_agent();
        agent.last_consent_response = Some(Instant::now() - Duration::from_secs(31));

        assert!(agent.is_consent_expired(), "consent should be expired after 30s");
        let timed_out = agent.check_consent_timeout();
        assert!(timed_out, "check_consent_timeout should return true");
        assert_eq!(
            agent.state(),
            IceConnectionState::Disconnected,
            "state should be Disconnected after consent expiry"
        );
        assert_eq!(agent.consent_failures, 1);
    }

    #[test]
    fn test_consent_not_expired_before_30s() {
        let mut agent = make_connected_agent();
        agent.last_consent_response = Some(Instant::now() - Duration::from_secs(20));

        assert!(!agent.is_consent_expired(), "consent should not be expired at 20s");
        let timed_out = agent.check_consent_timeout();
        assert!(!timed_out);
        assert_eq!(agent.state(), IceConnectionState::Connected);
    }

    #[test]
    fn test_build_consent_check_produces_valid_stun() {
        let mut agent = make_connected_agent();
        let (encoded, dest) = agent
            .build_consent_check()
            .unwrap_or_else(|e| panic!("build_consent_check: {e}"));

        assert_eq!(
            dest,
            "10.0.0.1:6000"
                .parse::<SocketAddr>()
                .unwrap_or_else(|e| panic!("parse: {e}"))
        );

        assert!(encoded.len() >= 20, "encoded STUN message too short");
        let decoded = StunMessage::decode(&encoded)
            .unwrap_or_else(|e| panic!("decode: {e}"));
        assert_eq!(decoded.msg_type, BINDING_REQUEST);

        let has_username = decoded.attributes.iter().any(|a| {
            if let StunAttribute::Username(u) = a {
                u.starts_with("REMT:")
            } else {
                false
            }
        });
        assert!(has_username, "consent check should contain USERNAME");

        let has_use_candidate = decoded
            .attributes
            .iter()
            .any(|a| matches!(a, StunAttribute::UseCandidate));
        assert!(
            !has_use_candidate,
            "consent check should NOT contain USE-CANDIDATE"
        );

        assert!(
            agent.pending_consent_txn.is_some(),
            "pending consent txn should be set"
        );
    }

    #[test]
    fn test_build_consent_check_fails_without_selected_pair() {
        let mut agent = IceAgent::new(IceRole::Controlling);
        agent.set_remote_credentials(
            "REMT".to_string(),
            "remote_password_22chars".to_string(),
        );
        agent.state = IceConnectionState::Connected;
        let result = agent.build_consent_check();
        assert!(result.is_err(), "should fail without selected pair");
    }

    #[test]
    fn test_handle_consent_response_ignores_wrong_txn() {
        let mut agent = make_connected_agent();
        agent.last_consent_response = Some(Instant::now() - Duration::from_secs(20));
        let _ = agent.build_consent_check();

        let wrong_txn = [0u8; 12];
        agent.handle_consent_response(&wrong_txn);

        assert!(
            agent.needs_consent_check(),
            "wrong txn should not reset consent timer"
        );
    }

    // --- Trickle ICE (RFC 8838) tests ---

    #[test]
    fn test_trickle_disabled_by_default() {
        let agent = IceAgent::new(IceRole::Controlling);
        assert!(!agent.is_trickle_enabled());
        assert!(!agent.has_end_of_candidates());
    }

    #[test]
    fn test_enable_trickle() {
        let mut agent = IceAgent::new(IceRole::Controlling);
        agent.enable_trickle();
        assert!(agent.is_trickle_enabled());
    }

    #[test]
    fn test_end_of_candidates() {
        let mut agent = IceAgent::new(IceRole::Controlling);
        assert!(!agent.has_end_of_candidates());
        agent.set_end_of_candidates();
        assert!(agent.has_end_of_candidates());
    }

    #[test]
    fn test_gather_host_candidates_only() {
        let mut agent = IceAgent::new(IceRole::Controlling);
        agent.enable_trickle();

        // Use a loopback address; gather_host_candidates filters out
        // loopback so we may get zero candidates, but the state should
        // transition to Gathering.
        let local_addr: SocketAddr = "127.0.0.1:0"
            .parse()
            .unwrap_or_else(|e| panic!("parse: {e}"));
        let _candidates = agent.gather_host_candidates_only(local_addr);

        assert_eq!(
            agent.state(),
            IceConnectionState::Gathering,
            "state should be Gathering after host-only gather"
        );
    }

    #[test]
    fn test_trickle_add_remote_candidate_during_checks() {
        let mut agent = IceAgent::new(IceRole::Controlling);
        agent.enable_trickle();
        agent.set_remote_credentials(
            "REMT".to_string(),
            "remote_password_22chars".to_string(),
        );

        // Add a local candidate manually
        agent.local_candidates.push(IceCandidate {
            foundation: "1".to_string(),
            component: ComponentId::Rtp,
            transport: "udp".to_string(),
            priority: 2130706431,
            address: "192.168.1.1:5000"
                .parse()
                .unwrap_or_else(|e| panic!("parse: {e}")),
            candidate_type: CandidateType::Host,
            related_address: None,
            ufrag: agent.local_credentials().ufrag.clone(),
        });

        // Add first remote candidate and start checks
        agent.add_remote_candidate(IceCandidate {
            foundation: "2".to_string(),
            component: ComponentId::Rtp,
            transport: "udp".to_string(),
            priority: 2130706431,
            address: "10.0.0.1:6000"
                .parse()
                .unwrap_or_else(|e| panic!("parse: {e}")),
            candidate_type: CandidateType::Host,
            related_address: None,
            ufrag: "REMT".to_string(),
        });

        let check_result = agent.start_checks();
        assert!(check_result.is_ok(), "should start checks");
        assert_eq!(agent.checklist().len(), 1);

        // Trickle: add another remote candidate while checks are running
        agent.add_remote_candidate(IceCandidate {
            foundation: "3".to_string(),
            component: ComponentId::Rtp,
            transport: "udp".to_string(),
            priority: 1694498815,
            address: "203.0.113.5:7000"
                .parse()
                .unwrap_or_else(|e| panic!("parse: {e}")),
            candidate_type: CandidateType::ServerReflexive,
            related_address: Some(
                "10.0.0.1:6000"
                    .parse()
                    .unwrap_or_else(|e| panic!("parse: {e}")),
            ),
            ufrag: "REMT".to_string(),
        });

        // Checklist should now have more pairs
        assert!(
            agent.checklist().len() >= 2,
            "checklist should grow after trickled candidate"
        );
    }
}
