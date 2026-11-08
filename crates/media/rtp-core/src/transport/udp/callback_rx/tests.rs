//! The lane-table receive entry against the async loop.
//!
//! The dual-run harness sends the same packet scripts through the async
//! receive loop (the switch off) and the lane-table entry (the switch on),
//! over real loopback sockets, and the traces must be identical: delivered
//! frames, the symmetric-RTP counters and latch, the loop's own drop counts,
//! the broadcast events, how the loop ended and where the SRTP context is
//! afterwards. Both drivers run the real `receive_loop` future; a
//! `ManualLane` stands in for the poller (the loop's future, then the ready
//! entries and the due fires, as a poller pass orders them).

use super::*;
use crate::lane_table::{callback_lane_counters, ManualLane};
use crate::packet::RtpHeader;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

const SSRC: u32 = 0x5eed_0062;
/// Passes per script step (each after a 1 ms reactor turn).
const SETTLE_PASSES: usize = 8;

/// Which receive driver a run uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Driver {
    /// The async loop (the switch off).
    Loop,
    /// A lane-table entry.
    Entry,
    /// A lane-table entry read on its lane epoll's readiness reports
    /// (Linux).
    Readiness,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Seen {
    source: SocketAddr,
    seq: u16,
    ts: u32,
    ssrc: u32,
    marker: bool,
    payload: Vec<u8>,
}

type Log = Arc<parking_lot::Mutex<Vec<Seen>>>;

/// The session's inline ingress: records every packet; `panic_at` panics on
/// that sequence number; `hold` asks for an owner deadline that long after
/// each packet and records when it was served.
struct Recording {
    log: Log,
    panic_at: Option<u16>,
    hold: Option<Duration>,
    pending: Vec<Instant>,
    served: Arc<parking_lot::Mutex<Vec<(Instant, Instant)>>>,
}

impl InlineRtpIngress for Recording {
    fn on_rtp(&mut self, packet: &InlineRtpPacket<'_>, now: Instant) {
        if self.panic_at == Some(packet.sequence_number) {
            panic!("scripted ingress panic");
        }
        self.log.lock().push(Seen {
            source: packet.source,
            seq: packet.sequence_number,
            ts: packet.timestamp,
            ssrc: packet.ssrc,
            marker: packet.marker,
            payload: packet.payload.to_vec(),
        });
        if let Some(hold) = self.hold {
            self.pending.push(now + hold);
        }
    }

    fn next_deadline(&mut self, _now: Instant) -> Option<Instant> {
        self.pending.iter().min().copied()
    }

    fn on_deadline(&mut self, now: Instant) {
        let mut served = self.served.lock();
        self.pending.retain(|deadline| {
            if *deadline <= now {
                served.push((*deadline, now));
                false
            } else {
                true
            }
        });
    }
}

enum Step {
    /// These datagrams from peer `.0`, sent together.
    Send(usize, Vec<Vec<u8>>),
    /// `stop_receiver`'s first half: `active` false, the doorbell rung.
    Close,
    /// A re-INVITE address change (`set_remote_rtp_addr`) to peer 1: a new
    /// latch generation.
    Readdress,
}

#[derive(Debug, PartialEq, Eq)]
struct Trace {
    /// Delivered frames in order: (peer, seq, ts, ssrc, marker, payload).
    seen: Vec<(usize, u16, u32, u32, bool, Vec<u8>)>,
    /// Learned, rebindings, probation packets, rejected packets.
    symmetric: (bool, u64, u64, u64),
    /// The latched remote (peer index) and the latch flag.
    remote: Option<usize>,
    latched: bool,
    /// The loop's SRTP failures, malformed RTP and non-RTP drops.
    drops: [u64; 3],
    /// Broadcast events: RTCP, STUN, DTMF, media, other.
    events: [usize; 5],
    /// The loop ended by itself (its `active` check).
    ended: bool,
    /// The inbound SRTP context is back home after the loop's state was
    /// dropped.
    srtp_home: bool,
    /// Sockets still registered with the lane epoll after the loop ended (0:
    /// unregistered on every exit).
    epoll_left: usize,
}

fn rtp_packet(pt: u8, seq: u16, payload: Vec<u8>) -> RtpPacket {
    RtpPacket::new(
        RtpHeader::new(pt, seq, u32::from(seq) * 160, SSRC),
        Bytes::from(payload),
    )
}

fn audio(seq: u16) -> Vec<u8> {
    (0..160u16)
        .map(|index| (index as u8) ^ (seq as u8))
        .collect()
}

fn rtp(seq: u16) -> Vec<u8> {
    rtp_packet(0, seq, audio(seq)).serialize().unwrap().to_vec()
}

/// A sender and receiver context sharing a key random to this process.
fn srtp_pair() -> (crate::srtp::SrtpContext, crate::srtp::SrtpContext) {
    use crate::srtp::{SrtpContext, SrtpCryptoKey, SRTP_AES128_CM_SHA1_80};
    static KEY: std::sync::OnceLock<([u8; 16], [u8; 14])> = std::sync::OnceLock::new();
    let (key, salt) = *KEY.get_or_init(|| (rand::random(), rand::random()));
    let context = || {
        SrtpContext::new(
            SRTP_AES128_CM_SHA1_80,
            SrtpCryptoKey::new(key.to_vec(), salt.to_vec()),
        )
        .unwrap()
    };
    (context(), context())
}

/// How a run's ingress behaves.
#[derive(Clone, Copy, Default)]
struct Options {
    srtp: bool,
    panic_at: Option<u16>,
    hold: Option<Duration>,
}

struct Run {
    socket: Arc<UdpSocket>,
    peers: Vec<UdpSocket>,
    active: Arc<AtomicBool>,
    doorbell: Arc<HotDoorbell>,
    generation: Arc<AtomicU64>,
    latched: Arc<AtomicBool>,
    remote: Arc<ArcSwapOption<SocketAddr>>,
    counters: Arc<SymmetricRtpCounters>,
    srtp_recv: Arc<SrtpRecvShared>,
    events: broadcast::Receiver<RtpEvent>,
    log: Log,
    served: Arc<parking_lot::Mutex<Vec<(Instant, Instant)>>>,
    lane: ManualLane,
    future: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    ended: bool,
    /// The most entries the lane's table held (1: the loop graduated).
    max_entries: usize,
    /// Receive handler calls the table made.
    rx_calls: u64,
}

impl Run {
    async fn new(driver: Driver, options: Options) -> Self {
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let mut peers = Vec::new();
        for _ in 0..2 {
            peers.push(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        }
        let (event_tx, events) = broadcast::channel(1024);
        let srtp_recv = Arc::new(SrtpRecvShared::new());
        if options.srtp {
            let mut slot = srtp_recv.lock();
            let _ = srtp_recv.install(&mut slot, srtp_pair().1);
        }
        let slot: Arc<HandoffSlot<Box<dyn InlineRtpIngress>>> = Arc::new(HandoffSlot::new());
        let log: Log = Arc::default();
        let served = Arc::new(parking_lot::Mutex::new(Vec::new()));
        assert!(slot
            .offer(Box::new(Recording {
                log: Arc::clone(&log),
                panic_at: options.panic_at,
                hold: options.hold,
                pending: Vec::new(),
                served: Arc::clone(&served),
            }))
            .is_ok());
        let active = Arc::new(AtomicBool::new(true));
        let doorbell = Arc::new(HotDoorbell::new());
        let generation = Arc::new(AtomicU64::new(0));
        let latched = Arc::new(AtomicBool::new(false));
        let remote = Arc::new(ArcSwapOption::from(None));
        let counters = Arc::new(SymmetricRtpCounters::default());
        let config = RxConfig {
            event_tx,
            active_state: Arc::clone(&active),
            srtp_recv: Arc::clone(&srtp_recv),
            secure_media_required: Arc::new(AtomicBool::new(options.srtp)),
            dtmf_seen: Arc::new(DashMap::new()),
            symmetric_rtp_policy: SymmetricRtpPolicy::default(),
            symmetric_rtp_counters: Arc::clone(&counters),
            symmetric_rtp_latched: Arc::clone(&latched),
            symmetric_rtp_generation: Arc::clone(&generation),
            remote_rtp_addr: Arc::clone(&remote),
            remote_rtcp_addr: Arc::new(ArcSwapOption::from(None)),
            rtcp_mux: Arc::new(AtomicBool::new(true)),
            latched_rtp_ssrc: Arc::new(AtomicU64::new(NO_LATCHED_SSRC)),
            hot_doorbell: Some(Arc::clone(&doorbell)),
            #[cfg(feature = "dtls-webrtc")]
            dtls_tx: Arc::new(parking_lot::Mutex::new(None)),
            srtp_diagnostics: false,
            rtp_diagnostics: false,
            local_rtp_addr: socket.local_addr().ok(),
            recv_buffer_size: 1500,
            inline_slot: slot,
            on_lane: true,
            owner_srtp: true,
        };
        let future: Pin<Box<dyn Future<Output = ()> + Send>> =
            Box::pin(receive_loop(ReceiveLoop {
                config,
                socket: Arc::clone(&socket),
                callback_kernel: driver != Driver::Loop,
                rx_readiness: driver == Driver::Readiness,
            }));
        Self {
            socket,
            peers,
            active,
            doorbell,
            generation,
            latched,
            remote,
            counters,
            srtp_recv,
            events,
            log,
            served,
            lane: ManualLane::new(tokio::time::Instant::now(), None),
            future: Some(future),
            ended: false,
            max_entries: 0,
            rx_calls: 0,
        }
    }

    fn peer_addr(&self, peer: usize) -> SocketAddr {
        self.peers[peer].local_addr().unwrap()
    }

    fn seqs(&self) -> Vec<u16> {
        self.log.lock().iter().map(|seen| seen.seq).collect()
    }

    /// One poller pass: the loop's future, then the table's ready entries
    /// and due fires.
    fn pass(&mut self) {
        self.pass_waking(&futures::task::noop_waker());
    }

    /// [`Self::pass`] with `poller` as the poller's waker (what the lane's
    /// wakes between passes reach).
    fn pass_waking(&mut self, poller: &std::task::Waker) {
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        let Run {
            lane,
            future,
            ended,
            ..
        } = self;
        let (_, rx_calls) = lane.pass_with_waker(tokio::time::Instant::now(), poller, || {
            let done = future
                .as_mut()
                .is_some_and(|running| running.as_mut().poll(&mut cx).is_ready());
            if done {
                *future = None;
                *ended = true;
            }
        });
        self.rx_calls += rx_calls;
        self.max_entries = self.max_entries.max(self.lane.len());
    }

    async fn settle(&mut self) {
        for _ in 0..SETTLE_PASSES {
            // Loopback datagrams are queued when the send returns; the
            // reactor turn here reports their readiness (the entry's waker).
            tokio::time::sleep(Duration::from_millis(1)).await;
            self.pass();
        }
    }

    async fn step(&mut self, step: &Step) {
        match step {
            Step::Send(peer, datagrams) => {
                let to = self.socket.local_addr().unwrap();
                for datagram in datagrams {
                    self.peers[*peer].send_to(datagram, to).await.unwrap();
                }
                self.settle().await;
            }
            Step::Close => {
                self.active.store(false, Ordering::Release);
                self.doorbell.ring();
            }
            Step::Readdress => {
                let addr = self.peer_addr(1);
                self.latched.store(false, Ordering::Release);
                self.remote.store(Some(Arc::new(addr)));
                self.generation.fetch_add(1, Ordering::AcqRel);
                self.doorbell.ring();
            }
        }
    }

    /// `stop_receiver`'s abort (the loop's future dropped in a pass, on its
    /// lane) unless the loop ended, then the trace.
    fn finish(mut self) -> Trace {
        {
            let Run { lane, future, .. } = &mut self;
            lane.pass(tokio::time::Instant::now(), || drop(future.take()));
        }
        assert!(self.lane.is_empty(), "the entry left with its loop");
        let epoll_left = self.lane.rx_readiness_registered();
        let drops = rx_core::take_last_dropped_counts().expect("the loop's state was dropped");
        let mut events = [0usize; 5];
        loop {
            match self.events.try_recv() {
                Ok(event) => {
                    let index = match event {
                        RtpEvent::RtcpReceived { .. } => 0,
                        RtpEvent::StunPacket { .. } => 1,
                        RtpEvent::DtmfEvent { .. } => 2,
                        RtpEvent::MediaReceived { .. } => 3,
                        _ => 4,
                    };
                    events[index] += 1;
                }
                Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
        let peers: Vec<SocketAddr> = (0..2).map(|peer| self.peer_addr(peer)).collect();
        let index = |addr: SocketAddr| {
            peers
                .iter()
                .position(|peer| *peer == addr)
                .unwrap_or(usize::MAX)
        };
        let seen = self
            .log
            .lock()
            .iter()
            .map(|seen| {
                (
                    index(seen.source),
                    seen.seq,
                    seen.ts,
                    seen.ssrc,
                    seen.marker,
                    seen.payload.clone(),
                )
            })
            .collect();
        let symmetric = self.counters.snapshot();
        Trace {
            seen,
            symmetric: (
                symmetric.destination_learned,
                symmetric.accepted_rebindings,
                symmetric.probation_packets,
                symmetric.rejected_packets,
            ),
            remote: self.remote.load().as_deref().map(|addr| index(*addr)),
            latched: self.latched.load(Ordering::Acquire),
            drops,
            events,
            ended: self.ended,
            srtp_home: self.srtp_recv.lock().home.is_some(),
            epoll_left,
        }
    }
}

/// Run `script` through one driver; also returns whether the loop
/// graduated and the receive handler calls the table made.
async fn drive(driver: Driver, srtp: bool, script: &[Step]) -> (Trace, usize, u64) {
    let mut run = Run::new(
        driver,
        Options {
            srtp,
            ..Options::default()
        },
    )
    .await;
    // The loop's first poll.
    run.pass();
    for step in script {
        run.step(step).await;
    }
    let (entries, calls) = (run.max_entries, run.rx_calls);
    (run.finish(), entries, calls)
}

/// The scripts: (name, uses SRTP, steps). SRTP datagrams are protected once
/// here, so both drivers receive the same bytes.
fn scripts() -> Vec<(&'static str, bool, Vec<Step>)> {
    let seqs = |list: &[u16]| list.iter().map(|seq| rtp(*seq)).collect::<Vec<_>>();
    let mut malformed = rtp(60);
    malformed[0] = 0x8f; // 15 CSRCs announced, none present
    malformed.truncate(20);
    let rtcp_rr = vec![0x80, 201, 0x00, 0x01, 0x5e, 0xed, 0x00, 0x62];
    let mut stun = vec![0x00, 0x01, 0x00, 0x00, 0x21, 0x12, 0xa4, 0x42];
    stun.extend_from_slice(&[7u8; 12]);
    let dtmf = |seq: u16, end: bool| {
        let flags = if end { 0x80 | 10 } else { 10 };
        rtp_packet(101, seq, vec![5, flags, 0x00, 0xa0])
            .serialize()
            .unwrap()
            .to_vec()
    };
    // SRTP: in order, a tampered copy of 8 ahead of the real one, a replay
    // of 5, then more.
    let (mut sender, _) = srtp_pair();
    let protected: Vec<Vec<u8>> = (1..=15u16)
        .map(|seq| {
            sender
                .protect(&rtp_packet(0, seq, audio(seq)))
                .unwrap()
                .serialize()
                .unwrap()
                .to_vec()
        })
        .collect();
    let mut tampered = protected[7].clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0x5a;
    let srtp_steps = vec![
        Step::Send(0, protected[..7].to_vec()),
        Step::Send(0, vec![tampered, protected[7].clone()]),
        Step::Send(0, protected[8..10].to_vec()),
        Step::Send(0, vec![protected[4].clone()]),
        Step::Send(0, protected[10..].to_vec()),
    ];
    vec![
        (
            "in_order",
            false,
            vec![Step::Send(0, (100..130).map(rtp).collect())],
        ),
        (
            "reorder_duplicates_loss",
            false,
            vec![
                Step::Send(0, seqs(&[1, 3, 2, 2, 5, 4, 4, 9, 7, 8, 8, 12])),
                Step::Send(0, seqs(&[13, 15, 14, 20])),
            ],
        ),
        (
            "mixed_rtcp_stun_dtmf_and_junk",
            false,
            vec![
                Step::Send(0, seqs(&[1, 2])),
                Step::Send(
                    0,
                    vec![
                        rtcp_rr,
                        stun,
                        vec![0xff; 40],
                        vec![0x80],
                        malformed,
                        dtmf(3, false),
                        dtmf(4, true),
                        dtmf(4, true),
                        rtp(5),
                    ],
                ),
            ],
        ),
        ("srtp_replay_and_tamper", true, srtp_steps),
        (
            "symmetric_rtp_rebind",
            false,
            vec![
                Step::Send(0, (1..=10).map(rtp).collect()),
                Step::Send(1, (11..=20).map(rtp).collect()),
                // Back on the old source: on probation again.
                Step::Send(0, (21..=22).map(rtp).collect()),
            ],
        ),
        (
            "readdress_mid_call",
            false,
            vec![
                Step::Send(0, (1..=5).map(rtp).collect()),
                Step::Readdress,
                Step::Send(1, (6..=10).map(rtp).collect()),
            ],
        ),
        (
            "close_mid_burst",
            false,
            vec![
                Step::Send(0, (1..=5).map(rtp).collect()),
                Step::Close,
                Step::Send(0, (6..=25).map(rtp).collect()),
            ],
        ),
    ]
}

/// Identical receive, accounting, order and teardown, script by script.
#[tokio::test]
async fn kernel_receive_scripts_match_the_async_loop() {
    let before = callback_lane_counters();
    for (name, srtp, script) in scripts() {
        let (loop_trace, loop_entries, loop_calls) = drive(Driver::Loop, srtp, &script).await;
        let (entry_trace, entry_entries, entry_calls) = drive(Driver::Entry, srtp, &script).await;
        assert_eq!(
            entry_trace, loop_trace,
            "{name}: the entry received as the loop did"
        );
        assert_eq!(
            (loop_entries, loop_calls),
            (0, 0),
            "{name}: the loop stays a future"
        );
        assert_eq!(entry_entries, 1, "{name}: the loop graduated");
        assert!(entry_calls > 0, "{name}: the table drove it");
        // Not vacuous: what each script must show.
        let seqs: Vec<u16> = loop_trace.seen.iter().map(|seen| seen.1).collect();
        match name {
            "in_order" => assert_eq!(seqs, (100..130).collect::<Vec<_>>()),
            "reorder_duplicates_loss" => assert_eq!(
                seqs,
                vec![1, 3, 2, 2, 5, 4, 4, 9, 7, 8, 8, 12, 13, 15, 14, 20],
                "arrival order, duplicates and gaps as sent"
            ),
            "mixed_rtcp_stun_dtmf_and_junk" => {
                assert_eq!(seqs, vec![1, 2, 5]);
                assert_eq!(
                    loop_trace.events[..3],
                    [1, 1, 2],
                    "RTCP, STUN, two DTMF (one retransmit suppressed)"
                );
                assert_eq!(loop_trace.drops[1..], [1, 2], "malformed RTP, non-RTP");
            }
            "srtp_replay_and_tamper" => {
                assert_eq!(seqs, (1..=15).collect::<Vec<_>>());
                assert_eq!(loop_trace.drops[0], 2, "the tampered copy and the replay");
                assert!(
                    loop_trace.srtp_home,
                    "the context went home with the loop's state"
                );
            }
            "symmetric_rtp_rebind" => {
                assert_eq!(loop_trace.symmetric.1, 1, "one rebind");
                assert!(loop_trace.symmetric.2 > 0, "after probation");
            }
            "readdress_mid_call" => {
                assert_eq!(seqs, (1..=10).collect::<Vec<_>>());
                assert_eq!((loop_trace.remote, loop_trace.latched), (Some(1), true));
            }
            "close_mid_burst" => {
                assert_eq!(
                    seqs,
                    (1..=6).collect::<Vec<_>>(),
                    "one datagram after the close, then the loop's check"
                );
                assert!(loop_trace.ended);
            }
            _ => unreachable!(),
        }
        if name != "close_mid_burst" {
            assert!(!loop_trace.ended, "{name}: aborted, as stop_receiver does");
        }
    }
    let after = callback_lane_counters();
    assert!(
        after.rx_installs >= before.rx_installs + scripts().len() as u64,
        "every entry run installed its loop"
    );
    assert!(
        after.rx_ready_calls > before.rx_ready_calls,
        "served from readiness"
    );
}

/// Owner deadlines: an entry's owner timer is filed on the table's wheel and
/// served there, never early, as the async loop's owner timer is.
#[tokio::test]
async fn an_entry_serves_its_owner_deadlines_never_early() {
    for driver in [Driver::Loop, Driver::Entry] {
        let before = callback_lane_counters();
        let mut run = Run::new(
            driver,
            Options {
                hold: Some(Duration::from_millis(5)),
                ..Options::default()
            },
        )
        .await;
        run.pass();
        for seq in 1..=8u16 {
            run.step(&Step::Send(0, vec![rtp(seq)])).await;
        }
        for _ in 0..2_000 {
            if run.served.lock().len() == 8 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
            run.pass();
        }
        let served = run.served.lock().clone();
        assert_eq!(served.len(), 8, "{driver:?}: every deadline served");
        for (deadline, at) in &served {
            assert!(at >= deadline, "{driver:?}: never early");
        }
        assert_eq!(run.seqs(), (1..=8).collect::<Vec<_>>());
        drop(run.finish());
        if driver == Driver::Entry {
            assert!(
                callback_lane_counters().rx_due_calls > before.rx_due_calls,
                "served from the table's wheel"
            );
        }
    }
}

/// A panic in one call's receive path (here its inline ingress) ends that
/// loop only; its neighbour on the same lane keeps receiving from the table.
#[tokio::test]
async fn a_panicking_call_ends_only_its_own_receive_loop() {
    panicking_call(Driver::Entry).await;
}

async fn panicking_call(driver: Driver) {
    let mut good = Run::new(driver, Options::default()).await;
    let mut bad = Run::new(
        driver,
        Options {
            panic_at: Some(7),
            ..Options::default()
        },
    )
    .await;
    // One lane for both: the good loop is polled in the bad run's passes
    // (its own lane is unused).
    let mut good_future = good.future.take();
    let to_bad = bad.socket.local_addr().unwrap();
    let to_good = good.socket.local_addr().unwrap();
    let panics_before = callback_lane_counters().panics;
    for seq in 1..=10u16 {
        bad.peers[0].send_to(&rtp(seq), to_bad).await.unwrap();
        good.peers[0].send_to(&rtp(seq), to_good).await.unwrap();
        for _ in 0..4 {
            tokio::time::sleep(Duration::from_millis(1)).await;
            let waker = futures::task::noop_waker();
            let mut cx = Context::from_waker(&waker);
            let Run {
                lane,
                future,
                ended,
                ..
            } = &mut bad;
            lane.pass(tokio::time::Instant::now(), || {
                if future
                    .as_mut()
                    .is_some_and(|running| running.as_mut().poll(&mut cx).is_ready())
                {
                    *future = None;
                    *ended = true;
                }
                if let Some(running) = good_future.as_mut() {
                    assert!(
                        running.as_mut().poll(&mut cx).is_pending(),
                        "the good loop runs on"
                    );
                }
            });
        }
    }
    assert!(bad.ended, "the poisoned loop ended");
    assert!(callback_lane_counters().panics > panics_before);
    assert_eq!(bad.seqs(), (1..=6).collect::<Vec<_>>(), "up to the panic");
    assert_eq!(
        good.seqs(),
        (1..=10).collect::<Vec<_>>(),
        "the neighbour received everything"
    );
    assert_eq!(bad.lane.len(), 1, "only the neighbour's entry is left");
    assert_eq!(
        bad.lane.rx_readiness_registered(),
        usize::from(driver == Driver::Readiness),
        "only the neighbour's socket is left on the lane epoll"
    );
    // Teardown of the neighbour inside a pass: retired synchronously.
    bad.lane
        .pass(tokio::time::Instant::now(), || drop(good_future.take()));
    assert!(bad.lane.is_empty());
    assert_eq!(bad.lane.rx_readiness_registered(), 0);
}

/// Fairness and budgets: a socket with two drain budgets and more queued is
/// served 50 datagrams per pass (the async loop's budget yield), its quiet
/// neighbour in the first pass, and nothing is lost or reordered.
#[tokio::test]
async fn a_flooding_socket_is_served_one_drain_budget_per_pass() {
    flooding_socket(Driver::Entry).await;
}

async fn flooding_socket(driver: Driver) {
    let mut quiet = Run::new(driver, Options::default()).await;
    let mut flood = Run::new(driver, Options::default()).await;
    let mut quiet_future = quiet.future.take();
    let to_flood = flood.socket.local_addr().unwrap();
    let to_quiet = quiet.socket.local_addr().unwrap();
    let waker = futures::task::noop_waker();
    // Both graduate on the flood run's lane.
    flood.peers[0].send_to(&rtp(1), to_flood).await.unwrap();
    quiet.peers[0].send_to(&rtp(1), to_quiet).await.unwrap();
    let pass =
        |flood: &mut Run, quiet_future: &mut Option<Pin<Box<dyn Future<Output = ()> + Send>>>| {
            let mut cx = Context::from_waker(&waker);
            let Run { lane, future, .. } = flood;
            lane.pass_with_rx(tokio::time::Instant::now(), || {
                if let Some(running) = future.as_mut() {
                    let _ = running.as_mut().poll(&mut cx);
                }
                if let Some(running) = quiet_future.as_mut() {
                    let _ = running.as_mut().poll(&mut cx);
                }
            })
        };
    for _ in 0..6 {
        tokio::time::sleep(Duration::from_millis(1)).await;
        pass(&mut flood, &mut quiet_future);
    }
    assert_eq!(flood.lane.len(), 2, "both graduated");
    const FLOOD: u16 = 2 * INLINE_DRAIN_BUDGET as u16 + 10;
    for seq in 2..2 + FLOOD {
        flood.peers[0].send_to(&rtp(seq), to_flood).await.unwrap();
    }
    quiet.peers[0].send_to(&rtp(2), to_quiet).await.unwrap();
    // The reactor reports both sockets readable.
    tokio::time::sleep(Duration::from_millis(5)).await;
    let mut per_pass = Vec::new();
    for _ in 0..6 {
        let before = flood.log.lock().len();
        pass(&mut flood, &mut quiet_future);
        per_pass.push(flood.log.lock().len() - before);
        if per_pass.len() == 1 {
            assert_eq!(
                quiet.seqs(),
                vec![1, 2],
                "the quiet socket in the first pass"
            );
            assert!(flood.lane.rx_pending(), "the rest keeps the lane going");
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(&per_pass[..3], &[50, 50, 10], "one drain budget per pass");
    assert_eq!(
        flood.seqs(),
        (1..2 + FLOOD).collect::<Vec<_>>(),
        "nothing lost or reordered"
    );
    flood
        .lane
        .pass(tokio::time::Instant::now(), || drop(quiet_future.take()));
}

/// A real lane poller, parked between passes (no tick) and ticking, serves
/// graduated entries from their readiness (nothing lost or reordered), and
/// `stop_receiver`'s abort and join retire an entry and resolve once its
/// state is dropped.
#[tokio::test]
async fn a_poller_serves_graduated_entries_from_their_readiness() {
    poller_serves_entries(Driver::Entry).await;
}

async fn poller_serves_entries(driver: Driver) {
    use crate::lane_poller::{LanePoller, PollerTimers};
    let before = callback_lane_counters();
    for tick in [None, Some(Duration::from_millis(2))] {
        let mut loops = Vec::new();
        for _ in 0..3 {
            let mut run = Run::new(driver, Options::default()).await;
            let future = run.future.take().expect("not started");
            loops.push((run, future));
        }
        let timers = PollerTimers {
            lane_timer: true,
            grid: None,
            wheel: true,
            pass_clock: true,
            callback_table: true,
        };
        let poller = match tick {
            None => LanePoller::start_with_timers(&tokio::runtime::Handle::current(), timers),
            Some(quantum) => LanePoller::start_with_tick_and_timers(
                &tokio::runtime::Handle::current(),
                crate::lane_tick::TickScheduler::new(crate::lane_tick::TickGrid::new(
                    std::time::Instant::now(),
                    quantum,
                )),
                timers,
            ),
        };
        let mut hosted = Vec::new();
        for (run, future) in loops {
            let handle = poller.host(future).ok().expect("poller is running");
            hosted.push((run, handle));
        }
        for seq in 1..=20u16 {
            for (run, _) in &hosted {
                let to = run.socket.local_addr().unwrap();
                run.peers[0].send_to(&rtp(seq), to).await.unwrap();
            }
            tokio::time::sleep(Duration::from_millis(3)).await;
        }
        for _ in 0..2_000 {
            if hosted.iter().all(|(run, _)| run.log.lock().len() == 20) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        for (run, _) in &hosted {
            assert_eq!(run.seqs(), (1..=20).collect::<Vec<_>>(), "{tick:?}");
        }
        for (_, handle) in hosted {
            handle.abort();
            tokio::time::timeout(Duration::from_secs(30), handle.join())
                .await
                .expect("an aborted graduated loop joins");
        }
        drop(poller);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let after = callback_lane_counters();
    assert!(
        after.rx_installs >= before.rx_installs + 6,
        "every loop graduated"
    );
    if driver == Driver::Readiness {
        assert!(
            after.rx_epoll_registered >= before.rx_epoll_registered + 6,
            "every socket registered with its lane epoll"
        );
        assert!(
            after.rx_readable_calls > before.rx_readable_calls,
            "served from lane epoll reports"
        );
    } else {
        assert!(
            after.rx_ready_calls > before.rx_ready_calls,
            "served from readiness"
        );
    }
    assert!(
        after.rx_retired >= before.rx_retired + 6,
        "every entry retired by its abort"
    );
}

/// Single datagrams, each on its own readiness edge, through `driver`: the
/// sequence numbers delivered and the reads that found the socket empty.
async fn one_datagram_per_edge(driver: Driver, sends: u16) -> (Vec<u16>, u64) {
    let empty_before = empty_reads_on_this_thread();
    let mut run = Run::new(driver, Options::default()).await;
    run.pass();
    for seq in 1..=sends {
        run.step(&Step::Send(0, vec![rtp(seq)])).await;
    }
    let seqs = run.seqs();
    drop(run.finish());
    (seqs, empty_reads_on_this_thread() - empty_before)
}

/// The cost the entry keeps from the async loop: with one datagram per
/// readiness edge, both read until the socket is empty, so every edge after
/// the first datagram (taken before the inline ingress exists, with a plain
/// awaited receive) ends with a read that finds nothing.
#[tokio::test]
async fn the_async_loop_and_the_entry_end_each_readiness_edge_with_an_empty_read() {
    const SENDS: u16 = 24;
    let expected: Vec<u16> = (1..=SENDS).collect();
    for driver in [Driver::Loop, Driver::Entry] {
        let (seqs, empty) = one_datagram_per_edge(driver, SENDS).await;
        assert_eq!(seqs, expected, "{driver:?}");
        assert!(
            empty >= u64::from(SENDS - 1),
            "{driver:?}: {empty} empty reads"
        );
    }
}

// ---- lane readiness (Linux) ----

/// Every script through the async loop, the entry and the entry on lane
/// readiness gives the same trace (frames, accounting, events, how the loop
/// ended, the SRTP context home, no socket left on the lane epoll), and the
/// readiness runs really were served from their lane epoll's reports.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn lane_readiness_receive_scripts_match_the_async_loop_and_the_entry() {
    let before = callback_lane_counters();
    let scripts = scripts();
    for (name, srtp, script) in &scripts {
        let (loop_trace, loop_entries, _) = drive(Driver::Loop, *srtp, script).await;
        let (entry_trace, entry_entries, _) = drive(Driver::Entry, *srtp, script).await;
        let (ready_trace, ready_entries, ready_calls) =
            drive(Driver::Readiness, *srtp, script).await;
        assert_eq!(
            ready_trace, loop_trace,
            "{name}: lane readiness received as the async loop did"
        );
        assert_eq!(
            ready_trace, entry_trace,
            "{name}: lane readiness received as the entry did"
        );
        assert_eq!((loop_entries, entry_entries, ready_entries), (0, 1, 1));
        assert!(ready_calls > 0, "{name}: the table drove it");
        assert!(!ready_trace.seen.is_empty(), "{name}: not vacuous");
        assert_eq!(ready_trace.epoll_left, 0, "{name}: unregistered");
    }
    let after = callback_lane_counters();
    assert!(
        after.rx_epoll_registered >= before.rx_epoll_registered + scripts.len() as u64,
        "each readiness loop's socket was registered with its lane epoll"
    );
    assert!(
        after.rx_readable_calls > before.rx_readable_calls,
        "served from lane epoll reports"
    );
}

/// The point of lane readiness: with one datagram per readiness edge the
/// async loop and the entry end each edge with an empty read, while lane
/// readiness reads each datagram exactly once and never finds the socket
/// empty. Its only empty probe is one `epoll_wait` per pass that read.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn lane_readiness_reads_each_datagram_once_without_an_empty_read() {
    const SENDS: u16 = 24;
    let expected: Vec<u16> = (1..=SENDS).collect();
    let (loop_seqs, loop_empty) = one_datagram_per_edge(Driver::Loop, SENDS).await;
    let (entry_seqs, entry_empty) = one_datagram_per_edge(Driver::Entry, SENDS).await;
    let before = callback_lane_counters();
    let (ready_seqs, ready_empty) = one_datagram_per_edge(Driver::Readiness, SENDS).await;
    let after = callback_lane_counters();
    assert_eq!(loop_seqs, expected);
    assert_eq!(entry_seqs, expected);
    assert_eq!(ready_seqs, expected);
    assert!(
        loop_empty >= u64::from(SENDS - 1),
        "the async loop: {loop_empty}"
    );
    assert!(
        entry_empty >= u64::from(SENDS - 1),
        "the entry: {entry_empty}"
    );
    assert_eq!(
        ready_empty, 0,
        "lane readiness: no read found the socket empty"
    );
    // Process totals (other tests add to them): at least this run's
    // reports were served, each with one read.
    assert!(after.rx_readable_calls >= before.rx_readable_calls + u64::from(SENDS - 1));
    assert!(after.rx_epoll_reports >= before.rx_epoll_reports + u64::from(SENDS - 1));
}

/// No lost wakeup: a datagram that lands after a pass's last (empty) lane
/// epoll wait, before the reactor's readiness for the epoll is cleared and
/// its waker registered again, still wakes the lane and is read in the next
/// pass. Nothing marks the entry: only the reactor's report of the epoll
/// descriptor can deliver it.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn lane_readiness_receives_a_datagram_between_the_drain_and_the_rearm() {
    use std::sync::atomic::AtomicUsize;
    struct Count(AtomicUsize);
    impl std::task::Wake for Count {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let mut run = Run::new(Driver::Readiness, Options::default()).await;
    let to = run.socket.local_addr().unwrap();
    // One peer for every datagram (the first latches it).
    let peer = Arc::new(std::net::UdpSocket::bind("127.0.0.1:0").unwrap());
    run.pass();
    peer.send_to(&rtp(1), to).unwrap();
    run.settle().await;
    assert_eq!(run.max_entries, 1, "graduated");
    assert_eq!(run.lane.rx_readiness_registered(), 1);
    peer.send_to(&rtp(2), to).unwrap();
    // The reactor turns and records the epoll descriptor readable.
    tokio::time::sleep(Duration::from_millis(2)).await;
    let fired = Arc::new(AtomicUsize::new(0));
    {
        let (peer, fired) = (Arc::clone(&peer), Arc::clone(&fired));
        lane_table::set_after_empty_wait(Some(Box::new(move || {
            if fired.fetch_add(1, Ordering::SeqCst) == 0 {
                peer.send_to(&rtp(3), to).unwrap();
            }
        })));
    }
    let count = Arc::new(Count(AtomicUsize::new(0)));
    let poller = std::task::Waker::from(Arc::clone(&count));
    run.pass_waking(&poller);
    lane_table::set_after_empty_wait(None);
    assert_eq!(fired.load(Ordering::SeqCst), 1, "the window was hit once");
    assert_eq!(run.seqs(), vec![1, 2], "datagram 3 arrived after the drain");
    assert!(!run.lane.rx_pending(), "nothing marked: the lane may park");
    for _ in 0..10_000 {
        if count.0.load(Ordering::SeqCst) > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert!(
        count.0.load(Ordering::SeqCst) > 0,
        "the reactor woke the parked lane for datagram 3"
    );
    run.pass();
    assert_eq!(run.seqs(), vec![1, 2, 3], "read in the next pass");
    let trace = run.finish();
    assert_eq!(trace.epoll_left, 0);
}

/// The same panic isolation on lane readiness: the poisoned entry's socket
/// leaves the lane epoll with it, and the neighbour is still served from its
/// reports.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn lane_readiness_a_panicking_call_ends_only_its_own_receive_loop() {
    panicking_call(Driver::Readiness).await;
}

/// Lane readiness keeps the same budget: one read per lane epoll report and
/// at most `LANE_RX_READINESS_WAITS_PER_PASS` waits per pass, so a flooding
/// socket gets 50 reads per pass, its quiet neighbour is read in the first
/// wait, and the reports left keep the lane going.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn lane_readiness_serves_a_flooding_socket_one_drain_budget_per_pass() {
    assert_eq!(
        lane_table::LANE_RX_READINESS_WAITS_PER_PASS as usize,
        INLINE_DRAIN_BUDGET,
        "the per-pass wait budget is the async loop's drain budget"
    );
    flooding_socket(Driver::Readiness).await;
}

/// Lane readiness on a real lane, parked between passes (no tick) and
/// ticking: the lane epoll's reactor registration wakes the lane for every
/// datagram, nothing is lost or reordered, and every socket leaves the epoll
/// with its entry.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn lane_readiness_on_a_poller_serves_entries_from_lane_epoll_reports() {
    poller_serves_entries(Driver::Readiness).await;
}
