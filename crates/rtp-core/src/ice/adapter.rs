//! Adapter wrapping the production-grade `webrtc-ice` crate behind our existing
//! ICE API surface.
//!
//! This module provides [`IceAgentAdapter`] which delegates to
//! [`webrtc_ice::agent::Agent`] for full RFC 8445 compliance (aggressive
//! nomination, ICE restart, peer-reflexive candidates, etc.) while keeping
//! backward compatibility with the types in [`super::types`].
//!
//! # Why an adapter?
//!
//! Our self-built ICE agent (~3 000 LOC) covers roughly 45 % of RFC 8445.
//! `webrtc-ice` has 3.68 M downloads and full RFC compliance.  Rather than
//! rewriting every call-site we wrap the production crate and map between
//! its types and ours.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::sync::Mutex as TokioMutex;
use tracing::{debug, info, trace, warn};

use webrtc_ice::agent::agent_config::AgentConfig;
use webrtc_ice::agent::Agent as WebrtcAgent;
use webrtc_ice::candidate::candidate_host::CandidateHostConfig;
use webrtc_ice::candidate::candidate_base::CandidateBaseConfig;
use webrtc_ice::candidate::{Candidate, CandidateType as WCandidateType};
use webrtc_ice::state::ConnectionState as WConnectionState;
use webrtc_ice::url::{Url as IceUrl, SchemeType};

use crate::Error;
use crate::turn::TurnServerConfig;
use super::types::{
    CandidateType, CandidatePairState, ComponentId, IceCandidate,
    IceCandidatePair, IceConnectionState, IceCredentials, IceRole,
};

// ---------------------------------------------------------------------------
// Type-mapping helpers
// ---------------------------------------------------------------------------

/// Convert our [`IceRole`] to the `is_controlling` bool used by `webrtc-ice`.
fn role_to_controlling(role: IceRole) -> bool {
    matches!(role, IceRole::Controlling)
}

/// Map a `webrtc-ice` [`WConnectionState`] to our [`IceConnectionState`].
fn map_connection_state(ws: WConnectionState) -> IceConnectionState {
    match ws {
        WConnectionState::New => IceConnectionState::New,
        WConnectionState::Checking => IceConnectionState::Checking,
        WConnectionState::Connected => IceConnectionState::Connected,
        WConnectionState::Completed => IceConnectionState::Completed,
        WConnectionState::Failed => IceConnectionState::Failed,
        WConnectionState::Disconnected => IceConnectionState::Disconnected,
        WConnectionState::Closed => IceConnectionState::Closed,
        // Unspecified / future variants
        _ => IceConnectionState::New,
    }
}

/// Map a `webrtc-ice` [`WCandidateType`] to our [`CandidateType`].
fn map_candidate_type(ct: WCandidateType) -> CandidateType {
    match ct {
        WCandidateType::Host => CandidateType::Host,
        WCandidateType::ServerReflexive => CandidateType::ServerReflexive,
        WCandidateType::PeerReflexive => CandidateType::PeerReflexive,
        WCandidateType::Relay => CandidateType::Relay,
        _ => CandidateType::Host, // Unspecified fallback
    }
}

/// Map our [`CandidateType`] to a `webrtc-ice` [`WCandidateType`].
fn map_candidate_type_to_webrtc(ct: CandidateType) -> WCandidateType {
    match ct {
        CandidateType::Host => WCandidateType::Host,
        CandidateType::ServerReflexive => WCandidateType::ServerReflexive,
        CandidateType::PeerReflexive => WCandidateType::PeerReflexive,
        CandidateType::Relay => WCandidateType::Relay,
    }
}

/// Convert a `dyn Candidate` into our [`IceCandidate`] type.
fn candidate_to_ice_candidate(c: &dyn Candidate, ufrag: &str) -> IceCandidate {
    let component = if c.component() == 2 {
        ComponentId::Rtcp
    } else {
        ComponentId::Rtp
    };

    // Parse address + port from the candidate
    let addr_str = c.address();
    let port = c.port();
    let ip: std::net::IpAddr = addr_str.parse().unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
    let address = SocketAddr::new(ip, port);

    // Related address
    let related_address = c.related_address().map(|ra| {
        let ra_ip: std::net::IpAddr = ra.address.parse().unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
        SocketAddr::new(ra_ip, ra.port as u16)
    });

    IceCandidate {
        foundation: c.foundation(),
        component,
        transport: "udp".to_string(),
        priority: c.priority(),
        address,
        candidate_type: map_candidate_type(c.candidate_type()),
        related_address,
        ufrag: ufrag.to_string(),
    }
}

/// Build STUN/TURN [`IceUrl`] values from socket addresses and optional TURN configs.
fn build_ice_urls(
    stun_servers: &[SocketAddr],
    turn_configs: &[TurnServerConfig],
) -> Vec<IceUrl> {
    let mut urls = Vec::new();

    for stun in stun_servers {
        let url_str = format!("stun:{}:{}", stun.ip(), stun.port());
        match IceUrl::parse_url(&url_str) {
            Ok(u) => urls.push(u),
            Err(e) => warn!(url = %url_str, error = %e, "failed to parse STUN URL"),
        }
    }

    for turn in turn_configs {
        let url_str = format!("turn:{}:{}", turn.server.ip(), turn.server.port());
        match IceUrl::parse_url(&url_str) {
            Ok(mut u) => {
                u.username = turn.username.clone();
                u.password = turn.password.clone();
                urls.push(u);
            }
            Err(e) => warn!(url = %url_str, error = %e, "failed to parse TURN URL"),
        }
    }

    urls
}

// ---------------------------------------------------------------------------
// IceAgentAdapter
// ---------------------------------------------------------------------------

/// Production ICE agent backed by `webrtc-ice`.
///
/// Maintains the same public API surface as the self-built [`super::agent::IceAgent`]
/// so that call-sites in session-core can switch with minimal changes.
pub struct IceAgentAdapter {
    /// The webrtc-ice agent (created lazily or on `gather_candidates`).
    inner: Option<Arc<WebrtcAgent>>,

    /// Our role.
    role: IceRole,

    /// Local ICE credentials (generated eagerly for SDP).
    local_credentials: IceCredentials,

    /// Remote ICE credentials (set after signaling exchange).
    remote_credentials: Option<IceCredentials>,

    /// Gathered local candidates (copied from webrtc-ice for our API).
    local_candidates: Vec<IceCandidate>,

    /// Remote candidates we have been told about.
    remote_candidates: Vec<IceCandidate>,

    /// Current connection state (mirrored from webrtc-ice callbacks).
    state: Arc<TokioMutex<IceConnectionState>>,

    /// The selected (nominated) candidate pair.
    selected_pair: Arc<TokioMutex<Option<IceCandidatePair>>>,

    /// Component this adapter manages.
    component: ComponentId,

    /// Whether trickle ICE is enabled.
    trickle_enabled: bool,

    /// Whether end-of-candidates has been signalled.
    end_of_candidates: bool,

    /// Cancel sender for dial/accept.
    cancel_tx: Option<tokio::sync::mpsc::Sender<()>>,
}

impl IceAgentAdapter {
    /// Create a new adapter with the specified role.
    ///
    /// Generates random ICE credentials immediately (needed for SDP offers).
    pub fn new(role: IceRole) -> Self {
        let credentials = IceCredentials::generate();

        debug!(
            role = %role,
            ufrag = %credentials.ufrag,
            "created IceAgentAdapter (webrtc-ice backed)"
        );

        Self {
            inner: None,
            role,
            local_credentials: credentials,
            remote_credentials: None,
            local_candidates: Vec::new(),
            remote_candidates: Vec::new(),
            state: Arc::new(TokioMutex::new(IceConnectionState::New)),
            selected_pair: Arc::new(TokioMutex::new(None)),
            component: ComponentId::Rtp,
            trickle_enabled: false,
            end_of_candidates: false,
            cancel_tx: None,
        }
    }

    /// Create an adapter for a specific component.
    pub fn with_component(role: IceRole, component: ComponentId) -> Self {
        let mut adapter = Self::new(role);
        adapter.component = component;
        adapter
    }

    // ---------------------------------------------------------------
    // Accessors (mirror the old IceAgent API)
    // ---------------------------------------------------------------

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
    ///
    /// This reads from a mutex that is updated by the webrtc-ice callback.
    pub async fn state(&self) -> IceConnectionState {
        *self.state.lock().await
    }

    /// Synchronous state snapshot (best-effort, avoids async in non-async contexts).
    pub fn state_sync(&self) -> IceConnectionState {
        self.state.try_lock().map_or(IceConnectionState::New, |g| *g)
    }

    /// Get the selected (nominated) candidate pair.
    pub async fn selected_pair(&self) -> Option<IceCandidatePair> {
        self.selected_pair.lock().await.clone()
    }

    /// Synchronous selected-pair snapshot.
    pub fn selected_pair_sync(&self) -> Option<IceCandidatePair> {
        self.selected_pair.try_lock().ok().and_then(|g| g.clone())
    }

    /// Get all local candidates.
    pub fn local_candidates(&self) -> &[IceCandidate] {
        &self.local_candidates
    }

    /// Get the component this agent manages.
    pub fn component(&self) -> ComponentId {
        self.component
    }

    /// Set the remote ICE credentials received via signaling.
    pub fn set_remote_credentials(&mut self, ufrag: String, pwd: String) {
        debug!(
            remote_ufrag = %ufrag,
            "set remote ICE credentials (adapter)"
        );
        self.remote_credentials = Some(IceCredentials { ufrag, pwd });
    }

    // ---------------------------------------------------------------
    // Trickle ICE
    // ---------------------------------------------------------------

    /// Enable trickle ICE mode.
    pub fn enable_trickle(&mut self) {
        self.trickle_enabled = true;
        debug!("trickle ICE enabled (adapter)");
    }

    /// Returns whether trickle ICE is enabled.
    pub fn is_trickle_enabled(&self) -> bool {
        self.trickle_enabled
    }

    /// Mark end-of-candidates.
    pub fn set_end_of_candidates(&mut self) {
        self.end_of_candidates = true;
        info!("end-of-candidates signalled (adapter)");
    }

    /// Returns whether end-of-candidates has been signalled.
    pub fn has_end_of_candidates(&self) -> bool {
        self.end_of_candidates
    }

    // ---------------------------------------------------------------
    // Agent lifecycle
    // ---------------------------------------------------------------

    /// Build and initialise the underlying `webrtc-ice` [`Agent`].
    ///
    /// Called automatically by [`gather_candidates`], but can be called
    /// explicitly if you want to set up callbacks before gathering.
    async fn ensure_agent(
        &mut self,
        stun_servers: &[SocketAddr],
        turn_configs: &[TurnServerConfig],
    ) -> Result<Arc<WebrtcAgent>, Error> {
        if let Some(ref agent) = self.inner {
            return Ok(Arc::clone(agent));
        }

        let urls = build_ice_urls(stun_servers, turn_configs);

        let config = AgentConfig {
            urls,
            is_controlling: role_to_controlling(self.role),
            local_ufrag: self.local_credentials.ufrag.clone(),
            local_pwd: self.local_credentials.pwd.clone(),
            ..Default::default()
        };

        let agent = WebrtcAgent::new(config).await.map_err(|e| {
            Error::IceError(format!("failed to create webrtc-ice agent: {e}"))
        })?;

        let agent = Arc::new(agent);

        // Wire up state-change callback
        {
            let state_ref = Arc::clone(&self.state);
            agent.on_connection_state_change(Box::new(move |cs| {
                let state_ref = Arc::clone(&state_ref);
                Box::pin(async move {
                    let mapped = map_connection_state(cs);
                    debug!(
                        webrtc_state = %cs,
                        our_state = %mapped,
                        "ICE connection state changed (webrtc-ice)"
                    );
                    let mut guard = state_ref.lock().await;
                    *guard = mapped;
                })
            }));
        }

        // Wire up selected-pair callback
        {
            let pair_ref = Arc::clone(&self.selected_pair);
            let ufrag = self.local_credentials.ufrag.clone();
            agent.on_selected_candidate_pair_change(Box::new(move |local, remote| {
                let pair_ref = Arc::clone(&pair_ref);
                let ufrag = ufrag.clone();
                // Extract candidate data synchronously before the async block
                // to avoid lifetime issues with the borrowed references.
                let local_ic = candidate_to_ice_candidate(local.as_ref(), &ufrag);
                let remote_ic = candidate_to_ice_candidate(remote.as_ref(), &ufrag);
                Box::pin(async move {
                    info!(
                        local = %local_ic.address,
                        remote = %remote_ic.address,
                        "selected candidate pair changed (webrtc-ice)"
                    );
                    let pair = IceCandidatePair {
                        priority: IceCandidatePair::compute_priority(
                            local_ic.priority,
                            remote_ic.priority,
                        ),
                        local: local_ic,
                        remote: remote_ic,
                        state: CandidatePairState::Succeeded,
                        nominated: true,
                    };
                    let mut guard = pair_ref.lock().await;
                    *guard = Some(pair);
                })
            }));
        }

        self.inner = Some(Arc::clone(&agent));
        Ok(agent)
    }

    // ---------------------------------------------------------------
    // Candidate gathering
    // ---------------------------------------------------------------

    /// Gather only host candidates (fast, synchronous-ish for trickle).
    ///
    /// For trickle ICE the SDP offer goes out immediately with host
    /// candidates; STUN/TURN gathering happens in the background.
    pub fn gather_host_candidates_only(
        &mut self,
        local_addr: SocketAddr,
    ) -> Vec<IceCandidate> {
        // Delegate to the self-built gatherer for host-only (no network I/O).
        let host_candidates = super::gather::gather_host_candidates(
            local_addr,
            self.component,
            &self.local_credentials.ufrag,
        );

        for c in &host_candidates {
            debug!(candidate = %c, "gathered host candidate (adapter, trickle)");
        }
        self.local_candidates.extend(host_candidates.clone());
        host_candidates
    }

    /// Gather all local candidates (host + STUN server-reflexive + TURN relay).
    ///
    /// Creates the underlying `webrtc-ice` agent if not yet created, starts
    /// gathering, and collects candidates via the `on_candidate` callback.
    pub async fn gather_candidates(
        &mut self,
        local_addr: SocketAddr,
        stun_servers: &[SocketAddr],
    ) -> Result<Vec<IceCandidate>, Error> {
        self.gather_candidates_with_turn(local_addr, stun_servers, &[]).await
    }

    /// Gather candidates including TURN relay candidates.
    pub async fn gather_candidates_with_turn(
        &mut self,
        _local_addr: SocketAddr,
        stun_servers: &[SocketAddr],
        turn_configs: &[TurnServerConfig],
    ) -> Result<Vec<IceCandidate>, Error> {
        {
            let mut state = self.state.lock().await;
            *state = IceConnectionState::Gathering;
        }

        let agent = self.ensure_agent(stun_servers, turn_configs).await?;

        // Collect candidates via on_candidate callback
        let candidates: Arc<TokioMutex<Vec<IceCandidate>>> =
            Arc::new(TokioMutex::new(Vec::new()));
        let (done_tx, mut done_rx) = tokio::sync::mpsc::channel::<()>(1);
        let ufrag = self.local_credentials.ufrag.clone();

        {
            let candidates_ref = Arc::clone(&candidates);
            agent.on_candidate(Box::new(move |c| {
                let candidates_ref = Arc::clone(&candidates_ref);
                let done_tx = done_tx.clone();
                let ufrag = ufrag.clone();
                Box::pin(async move {
                    match c {
                        Some(candidate) => {
                            let ic = candidate_to_ice_candidate(candidate.as_ref(), &ufrag);
                            debug!(candidate = %ic, "gathered candidate (webrtc-ice)");
                            let mut guard = candidates_ref.lock().await;
                            guard.push(ic);
                        }
                        None => {
                            // Gathering complete
                            debug!("candidate gathering complete (webrtc-ice)");
                            let _ = done_tx.send(()).await;
                        }
                    }
                })
            }));
        }

        // Kick off gathering
        agent.gather_candidates().map_err(|e| {
            Error::IceError(format!("webrtc-ice gather_candidates failed: {e}"))
        })?;

        // Wait for gathering to finish (None sentinel) with a timeout
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            done_rx.recv(),
        )
        .await;

        // Collect results
        let gathered = {
            let guard = candidates.lock().await;
            guard.clone()
        };

        info!(
            count = gathered.len(),
            "candidate gathering complete (adapter)"
        );

        self.local_candidates = gathered.clone();
        Ok(gathered)
    }

    /// Add a remote candidate received via signaling.
    pub fn add_remote_candidate(&mut self, candidate: IceCandidate) {
        debug!(
            candidate = %candidate,
            "adding remote candidate (adapter)"
        );
        self.remote_candidates.push(candidate.clone());

        // Forward to webrtc-ice agent if it exists
        if let Some(ref agent) = self.inner {
            let agent = Arc::clone(agent);
            let candidate = candidate;
            tokio::spawn(async move {
                match build_webrtc_candidate(&candidate).await {
                    Ok(c) => {
                        if let Err(e) = agent.add_remote_candidate(&c) {
                            warn!(error = %e, "failed to add remote candidate to webrtc-ice");
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "failed to build webrtc-ice candidate from our type");
                    }
                }
            });
        }
    }

    /// Add multiple remote candidates at once.
    pub fn add_remote_candidates(&mut self, candidates: Vec<IceCandidate>) {
        for c in candidates {
            self.add_remote_candidate(c);
        }
    }

    /// Start connectivity checks by calling `dial` (controlling) or `accept`
    /// (controlled) on the underlying agent.
    ///
    /// This is the async equivalent of the old `start_checks()`.
    /// The webrtc-ice agent handles all check logic internally.
    pub async fn start_checks(&mut self) -> Result<(), Error> {
        let remote_creds = self.remote_credentials.as_ref().ok_or_else(|| {
            Error::IceError("cannot start checks: remote credentials not set".into())
        })?;

        let agent = self.inner.as_ref().ok_or_else(|| {
            Error::IceError("cannot start checks: agent not initialised (call gather_candidates first)".into())
        })?;

        // Set remote credentials on the webrtc-ice agent
        let remote_ufrag = remote_creds.ufrag.clone();
        let remote_pwd = remote_creds.pwd.clone();

        agent.set_remote_credentials(remote_ufrag.clone(), remote_pwd.clone()).await.map_err(|e| {
            Error::IceError(format!("failed to set remote credentials on webrtc-ice: {e}"))
        })?;

        // Create a cancel channel for dial/accept (webrtc-ice expects mpsc)
        let (cancel_tx, cancel_rx) = tokio::sync::mpsc::channel::<()>(1);
        self.cancel_tx = Some(cancel_tx);

        let agent = Arc::clone(agent);

        // Spawn the dial/accept in a background task -- it runs until a
        // candidate pair is selected or the cancel channel fires.
        // We split controlling/controlled into separate spawns to avoid
        // incompatible opaque return types from dial() vs accept().
        if self.role == IceRole::Controlling {
            tokio::spawn(async move {
                match agent.dial(cancel_rx, remote_ufrag, remote_pwd).await {
                    Ok(_conn) => {
                        info!("ICE connectivity established via dial (webrtc-ice)");
                    }
                    Err(e) => {
                        warn!(error = %e, "ICE dial failed (webrtc-ice)");
                    }
                }
            });
        } else {
            tokio::spawn(async move {
                match agent.accept(cancel_rx, remote_ufrag, remote_pwd).await {
                    Ok(_conn) => {
                        info!("ICE connectivity established via accept (webrtc-ice)");
                    }
                    Err(e) => {
                        warn!(error = %e, "ICE accept failed (webrtc-ice)");
                    }
                }
            });
        }

        info!(
            role = %self.role,
            "started ICE connectivity checks (webrtc-ice)"
        );

        Ok(())
    }

    /// Close the agent and release resources.
    pub async fn close(&mut self) {
        // Signal cancel to dial/accept by dropping the sender
        self.cancel_tx.take();

        if let Some(ref agent) = self.inner {
            if let Err(e) = agent.close().await {
                warn!(error = %e, "error closing webrtc-ice agent");
            }
        }

        self.inner = None;
        let mut state = self.state.lock().await;
        *state = IceConnectionState::Closed;
        debug!("IceAgentAdapter closed");
    }

    /// Synchronous close for non-async contexts (best-effort).
    pub fn close_sync(&mut self) {
        // Signal cancel to dial/accept by dropping the sender
        self.cancel_tx.take();
        // We cannot await agent.close() here, but dropping the Arc
        // will eventually clean up.
        self.inner = None;
        if let Ok(mut state) = self.state.try_lock() {
            *state = IceConnectionState::Closed;
        }
        debug!("IceAgentAdapter closed (sync)");
    }

    // ---------------------------------------------------------------
    // Consent freshness (RFC 7675) — handled internally by webrtc-ice
    // ---------------------------------------------------------------
    // webrtc-ice handles consent freshness and keepalives internally
    // via its `keepalive_interval` and `disconnected_timeout` config.
    // We expose no-op / passthrough methods so existing call-sites
    // compile unchanged.

    /// Returns whether a consent check is needed.
    ///
    /// webrtc-ice handles consent internally; this always returns `false`.
    pub fn needs_consent_check(&self) -> bool {
        false
    }

    /// Build a consent check (no-op for webrtc-ice adapter).
    pub fn build_consent_check(&self) -> Result<(Vec<u8>, SocketAddr), Error> {
        Err(Error::IceError(
            "consent checks are handled internally by webrtc-ice".into(),
        ))
    }

    /// Handle a consent response (no-op for webrtc-ice adapter).
    pub fn handle_consent_response(&mut self, _transaction_id: &[u8; 12]) {
        // webrtc-ice handles this internally
    }

    /// Returns whether consent has expired.
    ///
    /// webrtc-ice signals this via the Disconnected connection state.
    pub async fn is_consent_expired(&self) -> bool {
        let state = self.state.lock().await;
        matches!(*state, IceConnectionState::Disconnected | IceConnectionState::Failed)
    }

    /// Check consent timeout (delegates to state check).
    pub async fn check_consent_timeout(&self) -> bool {
        self.is_consent_expired().await
    }

    /// Get consent failure count (always 0; webrtc-ice manages internally).
    pub fn consent_failures(&self) -> u32 {
        0
    }

    // ---------------------------------------------------------------
    // Legacy compatibility shims
    // ---------------------------------------------------------------

    /// No-op checklist access (webrtc-ice manages its own checklist).
    ///
    /// Returns an empty slice; the real checklist lives inside webrtc-ice.
    pub fn checklist(&self) -> &[IceCandidatePair] {
        &[]
    }

    /// Next pair to check — not applicable with webrtc-ice (returns `None`).
    pub fn next_check(&self) -> Option<usize> {
        None
    }

    /// Check a pair by index — not applicable with webrtc-ice.
    pub fn check_pair(&self, _pair_idx: usize) -> Result<(Vec<u8>, SocketAddr), Error> {
        Err(Error::IceError(
            "manual check_pair not supported with webrtc-ice adapter".into(),
        ))
    }

    /// Handle an incoming STUN response — not applicable with webrtc-ice.
    pub fn handle_stun_response(&self, _response: &[u8], _from: SocketAddr) -> Result<(), Error> {
        // webrtc-ice handles STUN internally
        Ok(())
    }

    /// Handle an incoming STUN request — not applicable with webrtc-ice.
    pub fn handle_stun_request(
        &self,
        _request: &[u8],
        _from: SocketAddr,
        _local_addr: SocketAddr,
    ) -> Result<Option<Vec<u8>>, Error> {
        // webrtc-ice handles STUN requests internally
        Ok(None)
    }

    /// ICE restart using webrtc-ice's built-in restart support.
    pub async fn restart(&mut self) -> Result<(), Error> {
        let agent = self.inner.as_ref().ok_or_else(|| {
            Error::IceError("cannot restart: agent not initialised".into())
        })?;

        let new_creds = IceCredentials::generate();

        agent
            .restart(new_creds.ufrag.clone(), new_creds.pwd.clone())
            .await
            .map_err(|e| Error::IceError(format!("webrtc-ice restart failed: {e}")))?;

        self.local_credentials = new_creds;
        self.local_candidates.clear();
        self.remote_candidates.clear();

        info!("ICE restart complete (webrtc-ice)");
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helper: build a webrtc-ice candidate from our IceCandidate
// ---------------------------------------------------------------------------

/// Construct a `webrtc-ice` host candidate from our [`IceCandidate`].
///
/// This is intentionally limited to host candidates; server-reflexive and
/// relay candidates received from the remote side are communicated to
/// webrtc-ice via `add_remote_candidate` which only needs a host-style
/// object with the correct address/priority/component.
async fn build_webrtc_candidate(
    ic: &IceCandidate,
) -> Result<Arc<dyn Candidate + Send + Sync>, Error> {
    let config = CandidateHostConfig {
        base_config: CandidateBaseConfig {
            network: "udp".to_string(),
            address: ic.address.ip().to_string(),
            port: ic.address.port(),
            component: ic.component.id() as u16,
            priority: ic.priority,
            foundation: ic.foundation.clone(),
            ..Default::default()
        },
        ..Default::default()
    };

    let candidate = config.new_candidate_host().map_err(|e| {
        Error::IceError(format!("failed to create webrtc-ice host candidate: {e}"))
    })?;

    Ok(Arc::new(candidate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_role_to_controlling() {
        assert!(role_to_controlling(IceRole::Controlling));
        assert!(!role_to_controlling(IceRole::Controlled));
    }

    #[test]
    fn test_map_connection_state() {
        assert_eq!(map_connection_state(WConnectionState::New), IceConnectionState::New);
        assert_eq!(map_connection_state(WConnectionState::Checking), IceConnectionState::Checking);
        assert_eq!(map_connection_state(WConnectionState::Connected), IceConnectionState::Connected);
        assert_eq!(map_connection_state(WConnectionState::Completed), IceConnectionState::Completed);
        assert_eq!(map_connection_state(WConnectionState::Failed), IceConnectionState::Failed);
        assert_eq!(map_connection_state(WConnectionState::Disconnected), IceConnectionState::Disconnected);
        assert_eq!(map_connection_state(WConnectionState::Closed), IceConnectionState::Closed);
    }

    #[test]
    fn test_map_candidate_type() {
        assert_eq!(map_candidate_type(WCandidateType::Host), CandidateType::Host);
        assert_eq!(map_candidate_type(WCandidateType::ServerReflexive), CandidateType::ServerReflexive);
        assert_eq!(map_candidate_type(WCandidateType::PeerReflexive), CandidateType::PeerReflexive);
        assert_eq!(map_candidate_type(WCandidateType::Relay), CandidateType::Relay);
    }

    #[test]
    fn test_adapter_new() {
        let adapter = IceAgentAdapter::new(IceRole::Controlling);
        assert_eq!(adapter.role(), IceRole::Controlling);
        assert_eq!(adapter.state_sync(), IceConnectionState::New);
        assert!(adapter.selected_pair_sync().is_none());
        assert_eq!(adapter.local_credentials().ufrag.len(), 4);
        assert_eq!(adapter.local_credentials().pwd.len(), 22);
    }

    #[test]
    fn test_adapter_with_component() {
        let adapter = IceAgentAdapter::with_component(IceRole::Controlled, ComponentId::Rtcp);
        assert_eq!(adapter.component(), ComponentId::Rtcp);
        assert_eq!(adapter.role(), IceRole::Controlled);
    }

    #[test]
    fn test_set_remote_credentials() {
        let mut adapter = IceAgentAdapter::new(IceRole::Controlling);
        assert!(adapter.remote_credentials().is_none());

        adapter.set_remote_credentials("WXYZ".to_string(), "remote_pwd_22characters".to_string());
        let creds = adapter.remote_credentials();
        assert!(creds.is_some());
        let creds = creds.unwrap_or_else(|| panic!("should have credentials"));
        assert_eq!(creds.ufrag, "WXYZ");
        assert_eq!(creds.pwd, "remote_pwd_22characters");
    }

    #[test]
    fn test_trickle_disabled_by_default() {
        let adapter = IceAgentAdapter::new(IceRole::Controlling);
        assert!(!adapter.is_trickle_enabled());
        assert!(!adapter.has_end_of_candidates());
    }

    #[test]
    fn test_enable_trickle() {
        let mut adapter = IceAgentAdapter::new(IceRole::Controlling);
        adapter.enable_trickle();
        assert!(adapter.is_trickle_enabled());
    }

    #[test]
    fn test_end_of_candidates() {
        let mut adapter = IceAgentAdapter::new(IceRole::Controlling);
        adapter.set_end_of_candidates();
        assert!(adapter.has_end_of_candidates());
    }

    #[test]
    fn test_close_sync() {
        let mut adapter = IceAgentAdapter::new(IceRole::Controlling);
        adapter.close_sync();
        assert_eq!(adapter.state_sync(), IceConnectionState::Closed);
    }

    #[test]
    fn test_consent_noop() {
        let adapter = IceAgentAdapter::new(IceRole::Controlling);
        assert!(!adapter.needs_consent_check());
        assert_eq!(adapter.consent_failures(), 0);
        assert!(adapter.build_consent_check().is_err());
    }

    #[test]
    fn test_build_ice_urls_stun() {
        let stun: SocketAddr = "74.125.250.129:19302".parse().unwrap_or_else(|e| panic!("{e}"));
        let urls = build_ice_urls(&[stun], &[]);
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].scheme, SchemeType::Stun);
    }

    #[test]
    fn test_build_ice_urls_turn() {
        let turn_cfg = TurnServerConfig {
            server: "10.0.0.1:3478".parse().unwrap_or_else(|e| panic!("{e}")),
            username: "user".to_string(),
            password: "pass".to_string(),
        };
        let urls = build_ice_urls(&[], &[turn_cfg]);
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].scheme, SchemeType::Turn);
        assert_eq!(urls[0].username, "user");
        assert_eq!(urls[0].password, "pass");
    }

    #[test]
    fn test_checklist_empty() {
        let adapter = IceAgentAdapter::new(IceRole::Controlling);
        assert!(adapter.checklist().is_empty());
        assert!(adapter.next_check().is_none());
    }

    #[tokio::test]
    async fn test_start_checks_requires_credentials() {
        let mut adapter = IceAgentAdapter::new(IceRole::Controlling);
        let result = adapter.start_checks().await;
        assert!(result.is_err());
    }
}
