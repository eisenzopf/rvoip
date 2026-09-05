//! Scripted two-agent scenarios over a virtual wire.
//!
//! This file is the reason the agent is sans-io: a virtual clock advances in
//! 5 ms ticks, datagrams route through an optional port-restricted NAT, and
//! every pathology — loss, role conflicts, wrong passwords, vanished peers —
//! plays out identically on every run.

use rvoip_ice_core::stun::{parse, MessageClass};
use rvoip_ice_core::{
    AgentConfig, Candidate, Credentials, IceAgent, IceEvent, IceRole, IceState, Transmit,
};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

/// Decides whether one datagram survives the wire. Args: sender name, datagram.
type WireFilter = Box<dyn FnMut(&'static str, &Transmit) -> bool>;

/// A port-restricted NAT: inbound is admitted only from a (address, port)
/// the inside host has already sent to.
struct Nat {
    public_ip: std::net::IpAddr,
    next_port: u16,
    outbound_map: HashMap<SocketAddr, SocketAddr>,
    inbound_map: HashMap<SocketAddr, SocketAddr>,
    allowed: HashSet<(SocketAddr, SocketAddr)>,
}

impl Nat {
    fn new(public_ip: &str) -> Self {
        Self {
            public_ip: public_ip.parse().unwrap(),
            next_port: 40_000,
            outbound_map: HashMap::new(),
            inbound_map: HashMap::new(),
            allowed: HashSet::new(),
        }
    }

    fn outbound(&mut self, inside: SocketAddr, destination: SocketAddr) -> SocketAddr {
        let public = *self.outbound_map.entry(inside).or_insert_with(|| {
            let mapped = SocketAddr::new(self.public_ip, self.next_port);
            self.next_port += 1;
            mapped
        });
        self.inbound_map.insert(public, inside);
        self.allowed.insert((public, destination));
        public
    }

    fn inbound(&self, source: SocketAddr, public: SocketAddr) -> Option<SocketAddr> {
        if !self.allowed.contains(&(public, source)) {
            return None;
        }
        self.inbound_map.get(&public).copied()
    }

    /// Pre-allocate the mapping a STUN discovery would have learned.
    fn mapping_for(&mut self, inside: SocketAddr) -> SocketAddr {
        let server: SocketAddr = "192.0.2.250:3478".parse().unwrap();
        self.outbound(inside, server)
    }
}

struct Host {
    name: &'static str,
    agent: IceAgent,
    addr: SocketAddr,
    nat: Option<Nat>,
    events: Vec<IceEvent>,
    /// First transmission of each request, by transaction id. Retransmits
    /// reuse their id and are governed by RTO, not Ta, so they are recorded
    /// once. Nominations are flagged: Ta paces the generation of *ordinary*
    /// checks (RFC 8445 §14.2), not the nomination that follows a
    /// validation, so the pacing assertion excludes them.
    request_sends: Vec<(Instant, bool)>,
    seen_transactions: HashSet<String>,
}

impl Host {
    fn new(name: &'static str, addr: &str, config: AgentConfig) -> Self {
        let addr: SocketAddr = addr.parse().unwrap();
        let mut agent = IceAgent::new(config);
        agent.add_local_candidate(Candidate::host(addr, 1, 65_535));
        Self {
            name,
            agent,
            addr,
            nat: None,
            events: Vec::new(),
            request_sends: Vec::new(),
            seen_transactions: HashSet::new(),
        }
    }

    fn behind(mut self, nat: Nat) -> Self {
        self.nat = Some(nat);
        self
    }

    fn drain_events(&mut self) {
        while let Some(event) = self.agent.poll_event() {
            self.events.push(event);
        }
    }

    fn selected(&self) -> Option<(SocketAddr, SocketAddr)> {
        self.agent.selected_pair()
    }
}

struct Sim {
    a: Host,
    b: Host,
    now: Instant,
    /// Return false to drop the datagram.
    filter: WireFilter,
}

impl Sim {
    fn new(a: Host, b: Host) -> Self {
        Self {
            a,
            b,
            now: Instant::now(),
            filter: Box::new(|_, _| true),
        }
    }

    /// One 5 ms tick: timers fire, queues drain, packets route.
    fn tick(&mut self) {
        self.a.agent.handle_timeout(self.now);
        self.b.agent.handle_timeout(self.now);
        // Drain until quiescent so responses triggered by deliveries land in
        // the same tick, like a real sub-millisecond RTT would.
        loop {
            let mut moved = false;
            while let Some(transmit) = self.a.agent.poll_transmit() {
                moved = true;
                Self::route(
                    self.now,
                    &mut self.a,
                    &mut self.b,
                    transmit,
                    &mut self.filter,
                );
            }
            while let Some(transmit) = self.b.agent.poll_transmit() {
                moved = true;
                Self::route(
                    self.now,
                    &mut self.b,
                    &mut self.a,
                    transmit,
                    &mut self.filter,
                );
            }
            if !moved {
                break;
            }
        }
        self.a.drain_events();
        self.b.drain_events();
        self.now += Duration::from_millis(5);
    }

    fn route(
        now: Instant,
        from: &mut Host,
        to: &mut Host,
        transmit: Transmit,
        filter: &mut WireFilter,
    ) {
        if let Ok(message) = parse(&transmit.payload) {
            if message.class == MessageClass::Request {
                let id = format!("{:?}", message.transaction_id);
                if from.seen_transactions.insert(id) {
                    from.request_sends.push((now, message.use_candidate));
                }
            }
        }
        if !filter(from.name, &transmit) {
            return;
        }
        // Source address as the wire sees it.
        let wire_source = match from.nat.as_mut() {
            Some(nat) => nat.outbound(transmit.from, transmit.to),
            None => transmit.from,
        };
        // Destination resolution: direct host, or the peer's NAT mapping.
        if transmit.to == to.addr {
            let _ = to
                .agent
                .handle_packet(now, to.addr, wire_source, &transmit.payload);
            return;
        }
        if let Some(nat) = to.nat.as_ref() {
            if let Some(inside) = nat.inbound(wire_source, transmit.to) {
                let _ = to
                    .agent
                    .handle_packet(now, inside, wire_source, &transmit.payload);
            }
        }
        // Anything else is an unroutable address: silently dropped, exactly
        // like the internet would.
    }

    fn run_until(&mut self, deadline: Duration, mut done: impl FnMut(&Sim) -> bool) -> bool {
        let end = self.now + deadline;
        while self.now < end {
            self.tick();
            if done(self) {
                return true;
            }
        }
        false
    }
}

/// Exchange credentials and (by default) host candidates, as SDP would.
fn signal(a: &mut Host, b: &mut Host) {
    let a_creds = a.agent.local_credentials().clone();
    let b_creds = b.agent.local_credentials().clone();
    let a_candidates: Vec<Candidate> = a.agent.local_candidates().to_vec();
    let b_candidates: Vec<Candidate> = b.agent.local_candidates().to_vec();
    a.agent.set_remote_credentials(b_creds);
    b.agent.set_remote_credentials(a_creds);
    for candidate in b_candidates {
        a.agent.add_remote_candidate(candidate);
    }
    for candidate in a_candidates {
        b.agent.add_remote_candidate(candidate);
    }
}

fn creds(ufrag: &str, pwd: &str) -> Credentials {
    Credentials {
        ufrag: ufrag.into(),
        pwd: pwd.into(),
    }
}

fn full(role: IceRole, ufrag: &str, tie: u64) -> AgentConfig {
    let mut config = AgentConfig::full(role, creds(ufrag, &format!("{ufrag}-password-of-22ch")));
    config.tie_breaker = tie;
    config
}

#[test]
fn two_direct_agents_complete_and_agree() {
    let mut a = Host::new(
        "a",
        "198.51.100.1:5004",
        full(IceRole::Controlling, "aaaa", 10),
    );
    let mut b = Host::new(
        "b",
        "198.51.100.2:5004",
        full(IceRole::Controlled, "bbbb", 5),
    );
    signal(&mut a, &mut b);
    let mut sim = Sim::new(a, b);
    assert!(
        sim.run_until(Duration::from_secs(5), |sim| {
            sim.a.agent.state() == IceState::Completed && sim.b.agent.state() == IceState::Completed
        }),
        "both agents complete"
    );
    assert_eq!(
        sim.a.selected().unwrap(),
        (
            "198.51.100.1:5004".parse().unwrap(),
            "198.51.100.2:5004".parse().unwrap()
        )
    );
    assert_eq!(
        sim.b.selected().unwrap(),
        (
            "198.51.100.2:5004".parse().unwrap(),
            "198.51.100.1:5004".parse().unwrap()
        )
    );
}

#[test]
fn a_lite_server_admits_a_natted_full_client() {
    let server = Host::new(
        "server",
        "198.51.100.5:5004",
        AgentConfig::lite(creds("srvr", "server-password-of-22char")),
    );
    let mut nat = Nat::new("203.0.113.9");
    let client_inside: SocketAddr = "10.0.0.2:5004".parse().unwrap();
    let mapped = nat.mapping_for(client_inside);
    let mut client = Host::new(
        "client",
        "10.0.0.2:5004",
        full(IceRole::Controlling, "clnt", 99),
    )
    .behind(nat);
    // The client signals host + srflx, the server only its public host —
    // exactly what the SDP exchange would carry.
    client
        .agent
        .add_local_candidate(Candidate::server_reflexive(
            mapped,
            client_inside,
            "192.0.2.250:3478".parse().unwrap(),
            1,
            65_534,
        ));
    let mut server = server;
    signal(&mut client, &mut server);
    let mut sim = Sim::new(client, server);
    assert!(
        sim.run_until(Duration::from_secs(5), |sim| {
            sim.a.agent.state() == IceState::Completed && sim.b.agent.state() == IceState::Completed
        }),
        "client and lite server complete"
    );
    // The server must have selected the NAT mapping, not the unroutable
    // inside address the client also signaled.
    let (_, server_remote) = sim.b.selected().unwrap();
    assert_eq!(server_remote, mapped);
    assert!(sim
        .b
        .events
        .iter()
        .any(|event| matches!(event, IceEvent::Selected { .. })));
}

#[test]
fn a_lost_first_check_is_retransmitted_to_success() {
    let mut a = Host::new(
        "a",
        "198.51.100.1:5004",
        full(IceRole::Controlling, "aaaa", 10),
    );
    let mut b = Host::new(
        "b",
        "198.51.100.2:5004",
        full(IceRole::Controlled, "bbbb", 5),
    );
    signal(&mut a, &mut b);
    let mut sim = Sim::new(a, b);
    let drop_first_request = std::cell::Cell::new(true);
    sim.filter = Box::new(move |sender, transmit| {
        if sender == "a" && drop_first_request.get() {
            if let Ok(message) = parse(&transmit.payload) {
                if message.class == MessageClass::Request {
                    drop_first_request.set(false);
                    return false;
                }
            }
        }
        true
    });
    assert!(
        sim.run_until(Duration::from_secs(10), |sim| {
            sim.a.agent.state() == IceState::Completed && sim.b.agent.state() == IceState::Completed
        }),
        "retransmission recovers the lost check"
    );
}

#[test]
fn both_controlling_repairs_by_tiebreaker_and_completes() {
    let mut a = Host::new(
        "a",
        "198.51.100.1:5004",
        full(IceRole::Controlling, "aaaa", 100),
    );
    let mut b = Host::new(
        "b",
        "198.51.100.2:5004",
        full(IceRole::Controlling, "bbbb", 7),
    );
    signal(&mut a, &mut b);
    let mut sim = Sim::new(a, b);
    assert!(
        sim.run_until(Duration::from_secs(10), |sim| {
            sim.a.agent.state() == IceState::Completed && sim.b.agent.state() == IceState::Completed
        }),
        "role conflict repairs and completes"
    );
    let a_changed = sim
        .a
        .events
        .iter()
        .any(|e| matches!(e, IceEvent::RoleChanged(_)));
    let b_changed = sim
        .b
        .events
        .iter()
        .any(|e| matches!(e, IceEvent::RoleChanged(_)));
    assert!(
        a_changed ^ b_changed,
        "exactly one side must repair its role (a: {a_changed}, b: {b_changed})"
    );
    // The higher tie-breaker keeps control.
    assert_eq!(sim.a.agent.role(), IceRole::Controlling);
    assert_eq!(sim.b.agent.role(), IceRole::Controlled);
}

#[test]
fn an_unsignaled_natted_peer_is_learned_as_prflx() {
    let a = Host::new(
        "a",
        "198.51.100.1:5004",
        full(IceRole::Controlling, "aaaa", 10),
    );
    let nat = Nat::new("203.0.113.20");
    let b = Host::new("b", "10.0.0.7:5004", full(IceRole::Controlled, "bbbb", 5)).behind(nat);
    let mut a = a;
    let mut b = b;
    // b signals ONLY its unroutable inside address: a must discover the real
    // path from the source address of b's checks.
    signal(&mut a, &mut b);
    let mut sim = Sim::new(a, b);
    assert!(
        sim.run_until(Duration::from_secs(10), |sim| {
            sim.a.agent.state() == IceState::Completed && sim.b.agent.state() == IceState::Completed
        }),
        "prflx discovery completes"
    );
    let (_, a_remote) = sim.a.selected().unwrap();
    assert_eq!(
        a_remote,
        "203.0.113.20:40000".parse::<SocketAddr>().unwrap(),
        "a must select b's NAT mapping, never the 10.x address it signaled"
    );
}

#[test]
fn consent_expires_when_the_peer_vanishes() {
    let mut a = Host::new(
        "a",
        "198.51.100.1:5004",
        full(IceRole::Controlling, "aaaa", 10),
    );
    let mut b = Host::new(
        "b",
        "198.51.100.2:5004",
        full(IceRole::Controlled, "bbbb", 5),
    );
    signal(&mut a, &mut b);
    let mut sim = Sim::new(a, b);
    assert!(sim.run_until(Duration::from_secs(5), |sim| {
        sim.a.agent.state() == IceState::Completed && sim.b.agent.state() == IceState::Completed
    }));
    // The peer disappears: every subsequent datagram is lost.
    sim.filter = Box::new(|_, _| false);
    assert!(
        sim.run_until(Duration::from_secs(120), |sim| {
            sim.a
                .events
                .iter()
                .any(|event| matches!(event, IceEvent::ConsentExpired))
        }),
        "the controlling side notices the peer is gone"
    );
}

#[test]
fn restart_completes_a_second_time_with_new_credentials() {
    let mut a = Host::new(
        "a",
        "198.51.100.1:5004",
        full(IceRole::Controlling, "aaaa", 10),
    );
    let mut b = Host::new(
        "b",
        "198.51.100.2:5004",
        full(IceRole::Controlled, "bbbb", 5),
    );
    signal(&mut a, &mut b);
    let mut sim = Sim::new(a, b);
    assert!(sim.run_until(Duration::from_secs(5), |sim| {
        sim.a.agent.state() == IceState::Completed && sim.b.agent.state() == IceState::Completed
    }));

    // Re-INVITE with new ufrag/pwd on both sides (RFC 8445 §9).
    sim.a
        .agent
        .restart(creds("aaa2", "aaa2-password-of-22char!"));
    sim.b
        .agent
        .restart(creds("bbb2", "bbb2-password-of-22char!"));
    signal(&mut sim.a, &mut sim.b);
    assert!(
        sim.run_until(Duration::from_secs(10), |sim| {
            sim.a.agent.state() == IceState::Completed && sim.b.agent.state() == IceState::Completed
        }),
        "a restarted session completes again"
    );
    let selected_events = sim
        .a
        .events
        .iter()
        .filter(|event| matches!(event, IceEvent::Selected { .. }))
        .count();
    assert!(
        selected_events >= 2,
        "selection happened before and after restart"
    );
}

#[test]
fn ordinary_checks_are_paced_at_ta() {
    let mut a = Host::new(
        "a",
        "198.51.100.1:5004",
        full(IceRole::Controlling, "aaaa", 10),
    );
    // Several remote candidates so multiple ordinary checks queue up.
    let mut b = Host::new(
        "b",
        "198.51.100.2:5004",
        full(IceRole::Controlled, "bbbb", 5),
    );
    b.agent.add_local_candidate(Candidate::host(
        "198.51.100.2:5006".parse().unwrap(),
        1,
        65_534,
    ));
    b.agent.add_local_candidate(Candidate::host(
        "198.51.100.2:5008".parse().unwrap(),
        1,
        65_533,
    ));
    signal(&mut a, &mut b);
    let mut sim = Sim::new(a, b);
    sim.run_until(Duration::from_secs(3), |sim| {
        sim.a.agent.state() == IceState::Completed
    });
    // Ordinary checks only: retransmits are RTO-governed and the nomination
    // is a consequence of a validation, so neither is Ta's business.
    let ordinary: Vec<Instant> = sim
        .a
        .request_sends
        .iter()
        .filter(|(_, nomination)| !nomination)
        .map(|(at, _)| *at)
        .collect();
    assert!(
        ordinary.len() >= 3,
        "expected several ordinary checks, saw {}",
        ordinary.len()
    );
    for window in ordinary.windows(2) {
        let gap = window[1].saturating_duration_since(window[0]);
        assert!(
            gap >= Duration::from_millis(45),
            "two ordinary checks {gap:?} apart violate Ta pacing"
        );
    }
}

#[test]
fn wrong_passwords_never_validate_and_fail_closed() {
    let mut a = Host::new(
        "a",
        "198.51.100.1:5004",
        full(IceRole::Controlling, "aaaa", 10),
    );
    let mut b = Host::new(
        "b",
        "198.51.100.2:5004",
        full(IceRole::Controlled, "bbbb", 5),
    );
    // Deliberately cross the wires: both sides hold wrong peer passwords.
    let a_candidates: Vec<Candidate> = a.agent.local_candidates().to_vec();
    let b_candidates: Vec<Candidate> = b.agent.local_candidates().to_vec();
    a.agent
        .set_remote_credentials(creds("bbbb", "not-the-real-password-1"));
    b.agent
        .set_remote_credentials(creds("aaaa", "not-the-real-password-2"));
    for candidate in b_candidates {
        a.agent.add_remote_candidate(candidate);
    }
    for candidate in a_candidates {
        b.agent.add_remote_candidate(candidate);
    }
    let mut sim = Sim::new(a, b);
    assert!(
        sim.run_until(Duration::from_secs(30), |sim| {
            sim.a.agent.state() == IceState::Failed && sim.b.agent.state() == IceState::Failed
        }),
        "authentication failure must converge to Failed, not hang"
    );
    assert!(
        !sim.a
            .events
            .iter()
            .any(|e| matches!(e, IceEvent::PairValidated { .. })),
        "nothing may validate across a wrong password"
    );
}
