//! The RFC 8445 ICE agent, sans-io.
//!
//! The agent is a pure state machine: the io layer hands it inbound STUN
//! datagrams with [`IceAgent::handle_packet`] and the current time with
//! [`IceAgent::handle_timeout`], and polls [`IceAgent::poll_transmit`] for
//! datagrams to send, [`IceAgent::poll_event`] for decisions the application
//! must act on, and [`IceAgent::poll_timeout`] for when it next needs the
//! clock. Nothing here reads a clock or touches a socket, which is what
//! makes every pathology in this protocol — role conflicts, nomination
//! races, retransmission storms — a scripted deterministic test.
//!
//! Two deliberate simplifications, both documented where they bite:
//! single-component only (component 1; v1 requires rtcp-mux, and the
//! candidate model keeps component ids so growing out of this is an
//! extension), and a validated check pair whose mapped address differs from
//! the local candidate records the discovery on the pair rather than
//! constructing a separate peer-reflexive-local pair — for single-socket
//! UDP the send path is identical either way.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use crate::candidate::{pair_priority, prflx_priority, Candidate, CandidateKind};
use crate::stun::{parse, Attribute, MessageClass, ParsedMessage, StunMessage, TransactionId};

/// Short-term ICE credentials (RFC 8445 §5.3).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Credentials {
    /// Username fragment, at least 4 characters.
    pub ufrag: String,
    /// Password, at least 22 characters.
    pub pwd: String,
}

impl Credentials {
    /// Generate spec-sized random credentials.
    #[must_use]
    pub fn generate() -> Self {
        use rand::distributions::Alphanumeric;
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let take = |n: usize, rng: &mut rand::rngs::ThreadRng| -> String {
            (0..n).map(|_| char::from(rng.sample(Alphanumeric))).collect()
        };
        Self {
            ufrag: take(8, &mut rng),
            pwd: take(24, &mut rng),
        }
    }
}

/// Which side drives nomination (RFC 8445 §6.1.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IceRole {
    /// Runs nomination. The offerer, unless a conflict repairs it.
    Controlling,
    /// Follows the peer's nomination.
    Controlled,
}

/// Overall agent state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IceState {
    /// No remote candidates yet.
    New,
    /// Checks are running.
    Checking,
    /// At least one pair validated; media could flow.
    Connected,
    /// A pair is nominated; this is the working call path.
    Completed,
    /// Every pair failed. The application should fall back or hang up.
    Failed,
}

/// One datagram the io layer must send.
#[derive(Clone, Debug)]
pub struct Transmit {
    /// Local socket to send from (a host candidate's address).
    pub from: SocketAddr,
    /// Destination.
    pub to: SocketAddr,
    /// Encoded STUN message.
    pub payload: Vec<u8>,
}

/// Decisions the application must act on.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IceEvent {
    /// Overall state moved.
    StateChanged(IceState),
    /// A pair validated: media *can* flow here.
    PairValidated {
        /// Local socket of the validated pair.
        local_base: SocketAddr,
        /// Remote transport address.
        remote: SocketAddr,
    },
    /// A pair was nominated: retarget media here.
    Selected {
        /// Local socket to send media from.
        local_base: SocketAddr,
        /// Remote address to send media to.
        remote: SocketAddr,
    },
    /// A role conflict repaired our role (RFC 8445 §7.3.1.1).
    RoleChanged(IceRole),
    /// The peer stopped answering consent checks (RFC 7675): stop sending.
    ConsentExpired,
}

/// Tunables. The retransmit ladder deviates from RFC 8489's Rc=7 default —
/// deliberately, and configurably: seven doublings of 500 ms is over a
/// minute of waiting on a dead pair, which is not a defensible call-setup
/// budget. Six attempts with the RTO capped at 3 s concedes a pair in
/// ~12.5 s worst case.
#[derive(Clone, Debug)]
pub struct AgentConfig {
    /// Nomination role. Ignored for lite (always controlled).
    pub role: IceRole,
    /// RFC 8445 §2.5 lite mode: answer checks, never send them.
    pub lite: bool,
    /// Our short-term credentials.
    pub credentials: Credentials,
    /// 64-bit tie-breaker for role conflicts.
    pub tie_breaker: u64,
    /// Pacing interval between new checks (RFC 8445 §14: ≥ 50 ms).
    pub ta: Duration,
    /// Initial retransmission timeout.
    pub rto_initial: Duration,
    /// Retransmission timeout ceiling.
    pub rto_max: Duration,
    /// Attempts before a check concedes.
    pub max_retransmits: u32,
    /// How long after the first valid pair the controlling side waits for a
    /// better one before nominating.
    pub nomination_delay: Duration,
    /// Keepalive/consent cadence on the nominated pair.
    pub keepalive_interval: Duration,
    /// Silence on consent checks after which the peer is presumed gone.
    pub consent_expiry: Duration,
}

impl AgentConfig {
    /// A full agent.
    #[must_use]
    pub fn full(role: IceRole, credentials: Credentials) -> Self {
        Self {
            role,
            lite: false,
            credentials,
            tie_breaker: rand::Rng::gen(&mut rand::thread_rng()),
            ta: Duration::from_millis(50),
            rto_initial: Duration::from_millis(500),
            rto_max: Duration::from_secs(3),
            max_retransmits: 6,
            nomination_delay: Duration::from_millis(150),
            keepalive_interval: Duration::from_secs(15),
            consent_expiry: Duration::from_secs(30),
        }
    }

    /// A lite agent (public-address responder).
    #[must_use]
    pub fn lite(credentials: Credentials) -> Self {
        Self {
            lite: true,
            role: IceRole::Controlled,
            ..Self::full(IceRole::Controlled, credentials)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CheckState {
    Frozen,
    Waiting,
    InProgress,
    Succeeded,
    Failed,
}

#[derive(Debug)]
struct Pair {
    local: Candidate,
    remote: Candidate,
    priority: u64,
    state: CheckState,
    valid: bool,
    nominate_on_success: bool,
    /// What the peer's XOR-MAPPED-ADDRESS said our address was, when it
    /// differs from the local candidate (the prflx-local simplification).
    discovered_local: Option<SocketAddr>,
}

impl Pair {
    fn key(&self) -> (SocketAddr, SocketAddr) {
        (self.local.addr, self.remote.addr)
    }
    fn foundation(&self) -> String {
        format!("{}:{}", self.local.foundation, self.remote.foundation)
    }
}

#[derive(Debug)]
struct InFlight {
    transaction_id: TransactionId,
    pair: (SocketAddr, SocketAddr),
    payload: Vec<u8>,
    next_retransmit: Instant,
    rto: Duration,
    attempts: u32,
    use_candidate: bool,
    consent: bool,
}

#[derive(Debug)]
struct EarlyCheck {
    local: SocketAddr,
    source: SocketAddr,
    priority: Option<u32>,
    use_candidate: bool,
}

const MAX_PAIRS: usize = 64;
const MAX_EARLY_CHECKS: usize = 16;

/// The agent. See the module docs for the driving contract.
#[derive(Debug)]
pub struct IceAgent {
    config: AgentConfig,
    role: IceRole,
    remote_credentials: Option<Credentials>,
    local_candidates: Vec<Candidate>,
    remote_candidates: Vec<Candidate>,
    pairs: Vec<Pair>,
    triggered: VecDeque<(SocketAddr, SocketAddr)>,
    in_flight: Vec<InFlight>,
    transmits: VecDeque<Transmit>,
    events: VecDeque<IceEvent>,
    state: IceState,
    next_check_at: Option<Instant>,
    first_valid_at: Option<Instant>,
    nomination_sent: bool,
    selected: Option<(SocketAddr, SocketAddr)>,
    next_keepalive_at: Option<Instant>,
    last_consent_at: Option<Instant>,
    consent_expired: bool,
    early_checks: Vec<EarlyCheck>,
}

impl IceAgent {
    /// Build an agent. It emits nothing until candidates arrive.
    #[must_use]
    pub fn new(config: AgentConfig) -> Self {
        let role = if config.lite {
            IceRole::Controlled
        } else {
            config.role
        };
        Self {
            role,
            remote_credentials: None,
            local_candidates: Vec::new(),
            remote_candidates: Vec::new(),
            pairs: Vec::new(),
            triggered: VecDeque::new(),
            in_flight: Vec::new(),
            transmits: VecDeque::new(),
            events: VecDeque::new(),
            state: IceState::New,
            next_check_at: None,
            first_valid_at: None,
            nomination_sent: false,
            selected: None,
            next_keepalive_at: None,
            last_consent_at: None,
            consent_expired: false,
            early_checks: Vec::new(),
            config,
        }
    }

    /// Our credentials, for the SDP.
    #[must_use]
    pub fn local_credentials(&self) -> &Credentials {
        &self.config.credentials
    }

    /// Current role (conflicts can change it).
    #[must_use]
    pub const fn role(&self) -> IceRole {
        self.role
    }

    /// Overall state.
    #[must_use]
    pub const fn state(&self) -> IceState {
        self.state
    }

    /// The nominated (local socket, remote address), once selected.
    #[must_use]
    pub const fn selected_pair(&self) -> Option<(SocketAddr, SocketAddr)> {
        self.selected
    }

    /// Local candidates, for the SDP.
    #[must_use]
    pub fn local_candidates(&self) -> &[Candidate] {
        &self.local_candidates
    }

    /// Add a local candidate. Server-reflexive candidates are signaling-only
    /// (RFC 8445 §6.1.2.4 prunes their pairs down to the base), so only host
    /// candidates form pairs here.
    pub fn add_local_candidate(&mut self, candidate: Candidate) {
        if self
            .local_candidates
            .iter()
            .any(|existing| existing.addr == candidate.addr && existing.kind == candidate.kind)
        {
            return;
        }
        self.local_candidates.push(candidate);
        self.form_pairs();
    }

    /// Add a remote candidate learned from signaling.
    pub fn add_remote_candidate(&mut self, candidate: Candidate) {
        if self
            .remote_candidates
            .iter()
            .any(|existing| existing.addr == candidate.addr)
        {
            return;
        }
        self.remote_candidates.push(candidate);
        self.form_pairs();
    }

    /// Provide the peer's credentials (from its SDP). Checks received before
    /// this are answered immediately and their pairing deferred to here.
    pub fn set_remote_credentials(&mut self, credentials: Credentials) {
        self.remote_credentials = Some(credentials);
        let early = std::mem::take(&mut self.early_checks);
        for check in early {
            self.note_inbound_check(check.local, check.source, check.priority, check.use_candidate);
        }
    }

    /// Restart ICE (RFC 8445 §9): new credentials, all checks forgotten.
    /// The application keeps media on the old path until a new selection.
    pub fn restart(&mut self, credentials: Credentials) {
        self.config.credentials = credentials;
        self.remote_credentials = None;
        self.remote_candidates.clear();
        self.pairs.clear();
        self.triggered.clear();
        self.in_flight.clear();
        self.first_valid_at = None;
        self.nomination_sent = false;
        self.selected = None;
        self.next_keepalive_at = None;
        self.last_consent_at = None;
        self.consent_expired = false;
        self.set_state(IceState::Checking);
    }

    /// Ingest one STUN datagram that arrived on `local` from `source`.
    ///
    /// # Errors
    ///
    /// Returns the codec error for datagrams that are not valid STUN; the
    /// caller already demuxed, so this is diagnostic.
    pub fn handle_packet(
        &mut self,
        now: Instant,
        local: SocketAddr,
        source: SocketAddr,
        data: &[u8],
    ) -> Result<(), crate::stun::StunError> {
        let message = parse(data)?;
        match message.class {
            MessageClass::Request => self.process_request(now, local, source, &message),
            MessageClass::SuccessResponse | MessageClass::ErrorResponse => {
                self.process_response(now, source, &message);
            }
            MessageClass::Indication => {
                // Keepalive received; nothing to answer (RFC 8445 §11).
            }
        }
        Ok(())
    }

    fn process_request(
        &mut self,
        now: Instant,
        local: SocketAddr,
        source: SocketAddr,
        message: &ParsedMessage<'_>,
    ) {
        let respond = |class: MessageClass, attributes: Vec<Attribute>, key: Option<&[u8]>| {
            let mut reply = StunMessage::binding(class, message.transaction_id);
            for attribute in attributes {
                reply = reply.with(attribute);
            }
            Transmit {
                from: local,
                to: source,
                payload: reply.encode(key, true),
            }
        };

        // RFC 8489 §9.2.4 short-term validation order. Errors for missing
        // pieces go out unprotected — the point of 400 is that the request
        // was not verifiable.
        if !message.has_integrity || message.username.is_none() {
            self.transmits.push_back(respond(
                MessageClass::ErrorResponse,
                vec![Attribute::ErrorCode {
                    code: 400,
                    reason: "Bad Request".into(),
                }],
                None,
            ));
            return;
        }
        let username = message.username.unwrap_or_default();
        let expected_prefix = format!("{}:", self.config.credentials.ufrag);
        if !username.starts_with(&expected_prefix)
            || !message.verify_integrity(self.config.credentials.pwd.as_bytes())
        {
            self.transmits.push_back(respond(
                MessageClass::ErrorResponse,
                vec![Attribute::ErrorCode {
                    code: 401,
                    reason: "Unauthorized".into(),
                }],
                None,
            ));
            return;
        }

        // Role conflict repair (RFC 8445 §7.3.1.1).
        if !self.config.lite {
            if self.role == IceRole::Controlling {
                if let Some(their_tie) = message.controlling {
                    if self.config.tie_breaker >= their_tie {
                        self.transmits.push_back(respond(
                            MessageClass::ErrorResponse,
                            vec![Attribute::ErrorCode {
                                code: 487,
                                reason: "Role Conflict".into(),
                            }],
                            Some(self.config.credentials.pwd.as_bytes()),
                        ));
                        return;
                    }
                    self.role = IceRole::Controlled;
                    self.events.push_back(IceEvent::RoleChanged(self.role));
                }
            } else if let Some(their_tie) = message.controlled {
                if self.config.tie_breaker >= their_tie {
                    self.role = IceRole::Controlling;
                    self.events.push_back(IceEvent::RoleChanged(self.role));
                } else {
                    self.transmits.push_back(respond(
                        MessageClass::ErrorResponse,
                        vec![Attribute::ErrorCode {
                            code: 487,
                            reason: "Role Conflict".into(),
                        }],
                        Some(self.config.credentials.pwd.as_bytes()),
                    ));
                    return;
                }
            }
        }

        // The check is authentic: answer it.
        self.transmits.push_back(respond(
            MessageClass::SuccessResponse,
            vec![Attribute::XorMappedAddress(source)],
            Some(self.config.credentials.pwd.as_bytes()),
        ));

        if self.config.lite {
            // Lite adopts the controlling peer's nomination and needs no
            // pair bookkeeping of its own.
            if message.use_candidate && self.selected != Some((local, source)) {
                self.selected = Some((local, source));
                self.events.push_back(IceEvent::Selected {
                    local_base: local,
                    remote: source,
                });
                self.set_state(IceState::Completed);
            }
            return;
        }

        if self.remote_credentials.is_some() {
            self.note_inbound_check(local, source, message.priority, message.use_candidate);
        } else if self.early_checks.len() < MAX_EARLY_CHECKS {
            // Answered above; pairing waits for the peer's SDP.
            self.early_checks.push(EarlyCheck {
                local,
                source,
                priority: message.priority,
                use_candidate: message.use_candidate,
            });
        }
        let _ = now;
    }

    /// An authenticated check arrived: learn prflx, trigger, honor UC.
    fn note_inbound_check(
        &mut self,
        local: SocketAddr,
        source: SocketAddr,
        priority: Option<u32>,
        use_candidate: bool,
    ) {
        if !self.remote_candidates.iter().any(|c| c.addr == source) {
            let priority = priority.unwrap_or_else(|| prflx_priority(0, 1));
            self.remote_candidates
                .push(Candidate::peer_reflexive(source, source, 1, priority));
            self.form_pairs();
        }
        let key = (local, source);
        let Some(pair) = self.pairs.iter_mut().find(|pair| pair.key() == key) else {
            return;
        };
        match pair.state {
            CheckState::Succeeded => {
                if use_candidate && pair.valid {
                    self.select(key);
                }
            }
            CheckState::InProgress | CheckState::Waiting | CheckState::Frozen
            | CheckState::Failed => {
                // RFC 8445 §7.3.1.4: a failed pair is given another chance by
                // a triggered check; frozen/waiting are simply expedited.
                if use_candidate {
                    pair.nominate_on_success = true;
                }
                pair.state = CheckState::Waiting;
                if !self.triggered.contains(&key) {
                    self.triggered.push_back(key);
                }
            }
        }
    }

    fn process_response(&mut self, now: Instant, source: SocketAddr, message: &ParsedMessage<'_>) {
        let Some(position) = self
            .in_flight
            .iter()
            .position(|flight| flight.transaction_id == message.transaction_id)
        else {
            return;
        };
        // RFC 8445 §7.2.5.2.1: the response must come from the address the
        // request went to, or the check has not proven that path.
        if self.in_flight[position].pair.1 != source {
            return;
        }
        let Some(remote_pwd) = self
            .remote_credentials
            .as_ref()
            .map(|credentials| credentials.pwd.clone())
        else {
            return;
        };

        if message.class == MessageClass::ErrorResponse {
            // Only an authenticated 487 acts immediately (we and every
            // compliant peer protect it). Anything else — including the
            // deliberately unprotected 400/401 — is left to retransmission
            // exhaustion: acting on unauthenticated errors would let a
            // spoofed packet kill a live check.
            if message.error.as_ref().map(|(code, _)| *code) == Some(487)
                && message.verify_integrity(remote_pwd.as_bytes())
            {
                let flight = self.in_flight.remove(position);
                self.role = match self.role {
                    IceRole::Controlling => IceRole::Controlled,
                    IceRole::Controlled => IceRole::Controlling,
                };
                self.events.push_back(IceEvent::RoleChanged(self.role));
                if let Some(pair) = self.pairs.iter_mut().find(|p| p.key() == flight.pair) {
                    pair.state = CheckState::Waiting;
                    if !self.triggered.contains(&flight.pair) {
                        self.triggered.push_back(flight.pair);
                    }
                }
                if flight.use_candidate {
                    self.nomination_sent = false;
                }
            }
            return;
        }

        // Success responses are keyed with the same password the request
        // used: the peer's (RFC 8445 §7.2.2/§7.3.2). An unauthenticated
        // success is discarded with the flight left running, so a spoofed
        // packet cannot kill a check either.
        if !message.verify_integrity(remote_pwd.as_bytes()) {
            return;
        }
        let flight = self.in_flight.remove(position);

        if flight.consent {
            self.last_consent_at = Some(now);
            return;
        }

        let mapped = message.xor_mapped_address;
        let mut newly_selected = None;
        let mut validated = None;
        if let Some(pair) = self.pairs.iter_mut().find(|p| p.key() == flight.pair) {
            pair.state = CheckState::Succeeded;
            pair.valid = true;
            if let Some(mapped) = mapped {
                if mapped != pair.local.addr {
                    pair.discovered_local = Some(mapped);
                }
            }
            validated = Some((pair.local.addr, pair.remote.addr, pair.foundation()));
            if flight.use_candidate || pair.nominate_on_success {
                newly_selected = Some(flight.pair);
            }
        }
        let Some((local, remote_addr, foundation)) = validated else {
            return;
        };
        self.events.push_back(IceEvent::PairValidated {
            local_base: local,
            remote: remote_addr,
        });
        if self.state == IceState::Checking || self.state == IceState::New {
            self.set_state(IceState::Connected);
        }
        if self.first_valid_at.is_none() {
            self.first_valid_at = Some(now);
        }
        // Unfreeze the rest of the foundation group (RFC 8445 §7.2.5.3.3).
        for pair in &mut self.pairs {
            if pair.state == CheckState::Frozen && pair.foundation() == foundation {
                pair.state = CheckState::Waiting;
            }
        }
        if let Some(key) = newly_selected {
            self.select(key);
            self.next_keepalive_at = Some(now + self.config.keepalive_interval);
            self.last_consent_at = Some(now);
        }
    }

    fn select(&mut self, key: (SocketAddr, SocketAddr)) {
        if self.selected == Some(key) {
            return;
        }
        self.selected = Some(key);
        if let Some(pair) = self.pairs.iter_mut().find(|p| p.key() == key) {
            pair.nominate_on_success = false;
        }
        self.events.push_back(IceEvent::Selected {
            local_base: key.0,
            remote: key.1,
        });
        self.set_state(IceState::Completed);
    }

    /// Advance timers: pacing, retransmits, nomination, keepalives, consent.
    pub fn handle_timeout(&mut self, now: Instant) {
        self.retransmit_due(now);
        if !self.config.lite {
            self.pace_checks(now);
            self.maybe_nominate(now);
            self.keepalive_due(now);
        }
    }

    fn retransmit_due(&mut self, now: Instant) {
        let mut failed_pairs = Vec::new();
        let mut expired_consent = false;
        let mut retransmits = Vec::new();
        self.in_flight.retain_mut(|flight| {
            if now < flight.next_retransmit {
                return true;
            }
            if flight.attempts >= self.config.max_retransmits {
                if flight.consent {
                    expired_consent = true;
                } else {
                    failed_pairs.push((flight.pair, flight.use_candidate));
                }
                return false;
            }
            flight.attempts += 1;
            flight.rto = (flight.rto * 2).min(self.config.rto_max);
            flight.next_retransmit = now + flight.rto;
            retransmits.push(Transmit {
                from: flight.pair.0,
                to: flight.pair.1,
                payload: flight.payload.clone(),
            });
            true
        });
        self.transmits.extend(retransmits);
        for (key, was_nomination) in failed_pairs {
            if let Some(pair) = self.pairs.iter_mut().find(|p| p.key() == key) {
                pair.state = CheckState::Failed;
                pair.valid = false;
            }
            if was_nomination {
                // Try the next best valid pair instead of giving up.
                self.nomination_sent = false;
            }
            self.evaluate_failure();
        }
        if expired_consent && !self.consent_expired {
            self.consent_expired = true;
            self.events.push_back(IceEvent::ConsentExpired);
        }
    }

    fn pace_checks(&mut self, now: Instant) {
        if self.remote_credentials.is_none() {
            return;
        }
        if let Some(next) = self.next_check_at {
            if now < next {
                return;
            }
        }
        // Triggered checks outrank ordinary ones (RFC 8445 §6.1.4.2).
        let key = loop {
            match self.triggered.pop_front() {
                Some(key) => {
                    let eligible = self
                        .pairs
                        .iter()
                        .find(|pair| pair.key() == key)
                        .is_some_and(|pair| pair.state == CheckState::Waiting);
                    if eligible {
                        break Some(key);
                    }
                }
                None => {
                    break self
                        .pairs
                        .iter()
                        .filter(|pair| pair.state == CheckState::Waiting)
                        .max_by_key(|pair| pair.priority)
                        .map(Pair::key);
                }
            }
        };
        let Some(key) = key else {
            self.next_check_at = None;
            return;
        };
        self.send_check(now, key, false);
        self.next_check_at = Some(now + self.config.ta);
    }

    fn maybe_nominate(&mut self, now: Instant) {
        if self.role != IceRole::Controlling
            || self.nomination_sent
            || self.selected.is_some()
        {
            return;
        }
        let Some(first_valid) = self.first_valid_at else {
            return;
        };
        let settled = now >= first_valid + self.config.nomination_delay || self.concluded();
        if !settled {
            return;
        }
        let Some(best) = self
            .pairs
            .iter()
            .filter(|pair| pair.valid)
            .max_by_key(|pair| pair.priority)
            .map(Pair::key)
        else {
            return;
        };
        self.send_check(now, best, true);
        self.nomination_sent = true;
    }

    fn keepalive_due(&mut self, now: Instant) {
        let Some(selected) = self.selected else {
            return;
        };
        if let Some(last) = self.last_consent_at {
            if now.saturating_duration_since(last) > self.config.consent_expiry
                && !self.consent_expired
            {
                self.consent_expired = true;
                self.events.push_back(IceEvent::ConsentExpired);
            }
        }
        let Some(due) = self.next_keepalive_at else {
            return;
        };
        if now < due {
            return;
        }
        self.next_keepalive_at = Some(now + self.config.keepalive_interval);
        // RFC 7675 consent: an integrity-protected request, not a bare
        // indication — silence must be distinguishable from a dead path.
        self.send_check_on(now, selected, false, true);
    }

    fn send_check(&mut self, now: Instant, key: (SocketAddr, SocketAddr), use_candidate: bool) {
        if let Some(pair) = self.pairs.iter_mut().find(|p| p.key() == key) {
            if !use_candidate {
                pair.state = CheckState::InProgress;
            }
        }
        self.send_check_on(now, key, use_candidate, false);
    }

    fn send_check_on(
        &mut self,
        now: Instant,
        key: (SocketAddr, SocketAddr),
        use_candidate: bool,
        consent: bool,
    ) {
        let Some(remote) = self.remote_credentials.as_ref() else {
            return;
        };
        let local_preference = self
            .pairs
            .iter()
            .find(|pair| pair.key() == key)
            .map_or(0, |pair| ((pair.local.priority >> 8) & 0xFFFF) as u16);
        let transaction_id = TransactionId::random();
        let mut message = StunMessage::binding(MessageClass::Request, transaction_id)
            .with(Attribute::Username(format!(
                "{}:{}",
                remote.ufrag, self.config.credentials.ufrag
            )))
            .with(Attribute::Priority(prflx_priority(local_preference, 1)));
        message = match self.role {
            IceRole::Controlling => {
                let message = message.with(Attribute::IceControlling(self.config.tie_breaker));
                if use_candidate {
                    message.with(Attribute::UseCandidate)
                } else {
                    message
                }
            }
            IceRole::Controlled => message.with(Attribute::IceControlled(self.config.tie_breaker)),
        };
        let payload = message.encode(Some(remote.pwd.as_bytes()), true);
        self.transmits.push_back(Transmit {
            from: key.0,
            to: key.1,
            payload: payload.clone(),
        });
        self.in_flight.push(InFlight {
            transaction_id,
            pair: key,
            payload,
            next_retransmit: now + self.config.rto_initial,
            rto: self.config.rto_initial,
            attempts: 0,
            use_candidate,
            consent,
        });
    }

    /// The next datagram to put on the wire.
    pub fn poll_transmit(&mut self) -> Option<Transmit> {
        self.transmits.pop_front()
    }

    /// The next application-facing decision.
    pub fn poll_event(&mut self) -> Option<IceEvent> {
        self.events.pop_front()
    }

    /// When [`Self::handle_timeout`] next wants the clock. `now` is only a
    /// floor: work that is already due reports `now` itself rather than a
    /// fabricated deadline, keeping this function pure.
    #[must_use]
    pub fn poll_timeout(&self, now: Instant) -> Option<Instant> {
        let mut earliest: Option<Instant> = None;
        let mut consider = |candidate: Option<Instant>| {
            earliest = match (earliest, candidate) {
                (None, next) => next,
                (Some(current), Some(next)) => Some(current.min(next)),
                (current, None) => current,
            };
        };
        consider(self.in_flight.iter().map(|f| f.next_retransmit).min());
        if !self.config.lite {
            let work = self.remote_credentials.is_some()
                && (!self.triggered.is_empty()
                    || self.pairs.iter().any(|pair| pair.state == CheckState::Waiting));
            if work {
                consider(Some(self.next_check_at.unwrap_or(now)));
            }
            if self.role == IceRole::Controlling
                && !self.nomination_sent
                && self.selected.is_none()
            {
                consider(self.first_valid_at.map(|t| t + self.config.nomination_delay));
            }
            consider(self.next_keepalive_at);
            if self.selected.is_some() && !self.consent_expired {
                consider(self.last_consent_at.map(|t| t + self.config.consent_expiry));
            }
        }
        earliest
    }

    fn form_pairs(&mut self) {
        for local in self
            .local_candidates
            .iter()
            .filter(|candidate| candidate.kind == CandidateKind::Host)
        {
            for remote in &self.remote_candidates {
                if local.component != remote.component
                    || local.addr.is_ipv4() != remote.addr.is_ipv4()
                {
                    continue;
                }
                let key = (local.addr, remote.addr);
                if self.pairs.iter().any(|pair| pair.key() == key) {
                    continue;
                }
                let (g, d) = match self.role {
                    IceRole::Controlling => (local.priority, remote.priority),
                    IceRole::Controlled => (remote.priority, local.priority),
                };
                self.pairs.push(Pair {
                    local: local.clone(),
                    remote: remote.clone(),
                    priority: pair_priority(g, d),
                    state: CheckState::Frozen,
                    valid: false,
                    nominate_on_success: false,
                    discovered_local: None,
                });
            }
        }
        self.pairs.sort_by(|a, b| b.priority.cmp(&a.priority));
        if self.pairs.len() > MAX_PAIRS {
            self.pairs.truncate(MAX_PAIRS);
        }
        // Initial unfreezing (RFC 8445 §6.1.2.6): the best pair of each
        // foundation group runs; the rest wait for its verdict.
        let mut seen = std::collections::HashSet::new();
        for pair in &mut self.pairs {
            let active = matches!(
                pair.state,
                CheckState::Waiting | CheckState::InProgress | CheckState::Succeeded
            );
            if seen.insert(pair.foundation()) {
                if pair.state == CheckState::Frozen {
                    pair.state = CheckState::Waiting;
                }
            } else if active {
                seen.insert(pair.foundation());
            }
        }
        if !self.config.lite && self.state == IceState::New && !self.pairs.is_empty() {
            self.set_state(IceState::Checking);
        }
    }

    fn concluded(&self) -> bool {
        self.pairs.iter().all(|pair| {
            matches!(pair.state, CheckState::Succeeded | CheckState::Failed)
        }) && self.in_flight.iter().all(|flight| flight.consent)
            && self.triggered.is_empty()
    }

    fn evaluate_failure(&mut self) {
        if self.state == IceState::Failed || self.config.lite {
            return;
        }
        let all_failed = !self.pairs.is_empty()
            && self.pairs.iter().all(|pair| pair.state == CheckState::Failed);
        if all_failed && self.triggered.is_empty() && self.selected.is_none() {
            self.set_state(IceState::Failed);
        }
    }

    fn set_state(&mut self, state: IceState) {
        if self.state != state {
            self.state = state;
            self.events.push_back(IceEvent::StateChanged(state));
        }
    }
}
