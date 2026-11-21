//! The lane table with real RTP sessions on real lane pollers.
//!
//! [`PacedRtp`] is the in-tree reference implementation of a
//! [`LaneTxSource`]: a paced 20 ms RTP sender over the synchronous TX path
//! (`RtpSendHandle::try_send_now`) whose deferred frames escalate to the
//! awaited send. Its wire output is compared with the same frames sent by a
//! paced task (`media_clock::sleep_until` + `send_packet_with_pt`).
use super::*;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use bytes::Bytes;
use tokio::net::UdpSocket;
use tokio::runtime::Handle;

use crate::executor::{override_media_executor_config_on_this_thread, MediaExecutorConfig};
use crate::lane_poller::{LanePoller, PollerTimers};
use crate::lane_tick::{TickGrid, TickScheduler};
use crate::session::{RtpFrameNow, RtpSendHandle, RtpSession, RtpSessionConfig};
use crate::srtp::{SrtpContext, SrtpCryptoKey, SRTP_AES128_CM_SHA1_80};
use crate::transport::UdpRtpTransport;

const FRAME: Duration = Duration::from_millis(20);
const SAMPLES_PER_FRAME: u32 = 160;
const PCMU: u8 = 0;
/// Frames one fire sends at most (a late lane catches up in bounded bursts).
const FRAMES_PER_FIRE: usize = 4;

/// One random SRTP master key and salt per test process: generated at run
/// time rather than hard-coded, shared by every context a test builds so
/// both ends of a call agree.
fn test_srtp_key() -> &'static ([u8; 16], [u8; 14]) {
    use rand::Rng;
    static KEY: std::sync::OnceLock<([u8; 16], [u8; 14])> = std::sync::OnceLock::new();
    KEY.get_or_init(|| (rand::thread_rng().gen(), rand::thread_rng().gen()))
}

fn srtp_context() -> SrtpContext {
    SrtpContext::new(
        SRTP_AES128_CM_SHA1_80,
        SrtpCryptoKey::new(test_srtp_key().0.to_vec(), test_srtp_key().1.to_vec()),
    )
    .unwrap()
}

/// The frames of one call: distinct payloads, a marker on the first.
fn script(frames: usize, seed: u8) -> Arc<Vec<Vec<u8>>> {
    Arc::new(
        (0..frames)
            .map(|frame| vec![seed ^ (frame as u8); 160])
            .collect(),
    )
}

fn timestamp_of(frame: usize) -> u32 {
    (frame as u32).wrapping_mul(SAMPLES_PER_FRAME)
}

/// The in-tree reference lane TX source (see the module docs).
struct PacedRtp {
    sender: RtpSendHandle,
    frames: Arc<Vec<Vec<u8>>>,
    /// The next frame to send.
    cursor: usize,
    /// Frame 0's deadline.
    start: Instant,
    /// (first frame of the fire, its deadline, the pass reading).
    fires: Arc<parking_lot::Mutex<Vec<(usize, Instant, Instant)>>>,
    dropped: Arc<AtomicUsize>,
}

impl PacedRtp {
    fn deadline(&self, frame: usize) -> Instant {
        self.start + FRAME * frame as u32
    }

    fn frame(&self, frame: usize) -> RtpFrameNow<'_> {
        RtpFrameNow {
            timestamp: timestamp_of(frame),
            payload: &self.frames[frame],
            marker: frame == 0,
            payload_type: PCMU,
        }
    }

    fn finished(&self) -> bool {
        self.cursor >= self.frames.len()
    }

    /// The awaited send of the next frame (after an escalation, or as the
    /// fallback when there is no table).
    async fn send_next_awaited(&mut self) {
        let frame = self.cursor;
        self.sender
            .send_packet_with_pt(
                timestamp_of(frame),
                Bytes::copy_from_slice(&self.frames[frame]),
                frame == 0,
                PCMU,
            )
            .await
            .expect("awaited send");
        self.cursor += 1;
    }
}

impl LaneTxSource for PacedRtp {
    fn on_due(&mut self, now: Instant) -> LaneNext {
        let first = self.cursor;
        let mut end = first;
        while end < self.frames.len() && end - first < FRAMES_PER_FIRE && self.deadline(end) <= now
        {
            end += 1;
        }
        self.fires.lock().push((first, self.deadline(first), now));
        let total = end - first;
        let result = {
            let batch: Vec<RtpFrameNow<'_>> = (first..end).map(|frame| self.frame(frame)).collect();
            self.sender.try_send_now(&batch, false)
        };
        match result {
            Ok(report) => {
                self.cursor += report.sent + usize::from(report.error.is_some());
                if report.deferred(total) > 0 {
                    return LaneNext::Escalate(Escalation::SendDeferred);
                }
            }
            Err(_) => return LaneNext::Done,
        }
        if self.finished() {
            LaneNext::Done
        } else {
            LaneNext::At(self.deadline(self.cursor))
        }
    }
}

impl Drop for PacedRtp {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::AcqRel);
    }
}

/// How a supervisor's source ended.
#[derive(Debug, PartialEq, Eq)]
enum Ended {
    /// Every frame sent; `installs` table installs (0: async fallback).
    Done {
        installs: u32,
    },
    Retired,
    Lost,
}

/// The supervisor: installs the source from its first poll, sends an
/// escalated frame with the awaited send and installs it again; drives the
/// source as a paced loop when no table takes it.
async fn supervise(mut source: Box<PacedRtp>) -> Ended {
    let mut installs = 0;
    loop {
        let first = source.deadline(source.cursor);
        let mut handle = match install_tx(source, first) {
            Ok(handle) => handle,
            Err(back) => {
                let mut source = back.downcast::<PacedRtp>().ok().expect("ours");
                while !source.finished() {
                    crate::media_clock::sleep_until(source.deadline(source.cursor)).await;
                    source.send_next_awaited().await;
                }
                return Ended::Done { installs };
            }
        };
        installs += 1;
        match handle.finished().await {
            Some(LaneTxReturn::Done(_)) => return Ended::Done { installs },
            Some(LaneTxReturn::Escalated(back, Escalation::SendDeferred)) => {
                source = back.downcast::<PacedRtp>().ok().expect("ours");
                source.send_next_awaited().await;
                record_resume(!source.finished());
                if source.finished() {
                    return Ended::Done { installs };
                }
            }
            Some(LaneTxReturn::Retired(_)) => return Ended::Retired,
            _ => return Ended::Lost,
        }
    }
}

/// The task path: the same frames from a paced future.
async fn paced_task(sender: RtpSendHandle, frames: Arc<Vec<Vec<u8>>>, start: Instant) {
    for (frame, payload) in frames.iter().enumerate() {
        crate::media_clock::sleep_until(start + FRAME * frame as u32).await;
        sender
            .send_packet_with_pt(
                timestamp_of(frame),
                Bytes::copy_from_slice(payload),
                frame == 0,
                PCMU,
            )
            .await
            .expect("send");
    }
}

struct Call {
    session: RtpSession,
    sender: RtpSendHandle,
    peer: UdpSocket,
}

async fn call(srtp: bool) -> Call {
    let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let session = RtpSession::new_event_driven(RtpSessionConfig {
        local_addr: "127.0.0.1:0".parse().unwrap(),
        remote_addr: Some(peer.local_addr().unwrap()),
        ssrc: Some(0x5eed_0001),
        payload_type: PCMU,
        ..RtpSessionConfig::default()
    })
    .await
    .unwrap();
    if srtp {
        session
            .transport()
            .as_any()
            .downcast_ref::<UdpRtpTransport>()
            .expect("UDP session transport")
            .set_srtp_contexts(srtp_context(), srtp_context())
            .await
            .unwrap();
    }
    let sender = session.send_handle().expect("send handle");
    Call {
        session,
        sender,
        peer,
    }
}

/// (sequence offset from the first, timestamp, marker, payload type, SSRC,
/// payload) of every datagram, decrypted when `srtp`.
type Wire = Vec<(u16, u32, bool, u8, u32, Vec<u8>)>;

async fn receive(peer: &UdpSocket, count: usize, srtp: bool) -> Wire {
    let mut context = srtp.then(srtp_context);
    let mut buffer = [0u8; 1500];
    let mut packets = Vec::new();
    for _ in 0..count {
        let (size, _) = tokio::time::timeout(Duration::from_secs(5), peer.recv_from(&mut buffer))
            .await
            .expect("datagram arrives")
            .unwrap();
        let packet = match context.as_mut() {
            Some(context) => context.unprotect(&buffer[..size]).expect("authentic"),
            None => crate::packet::RtpPacket::parse(&buffer[..size]).unwrap(),
        };
        packets.push(packet);
    }
    let first = packets[0].header.sequence_number;
    packets
        .into_iter()
        .map(|packet| {
            (
                packet.header.sequence_number.wrapping_sub(first),
                packet.header.timestamp,
                packet.header.marker,
                packet.header.payload_type,
                packet.header.ssrc,
                packet.payload.to_vec(),
            )
        })
        .collect()
}

fn table_timers(grid: Option<TickGrid>) -> PollerTimers {
    PollerTimers {
        lane_timer: true,
        grid,
        wheel: true,
        pass_clock: true,
        callback_table: true,
    }
}

/// A ticking (2 ms, grid timers) or immediate poller with a table.
fn table_poller(tick: bool) -> LanePoller {
    if tick {
        let grid = TickGrid::new(std::time::Instant::now(), Duration::from_millis(2));
        LanePoller::start_with_tick_and_timers(
            &Handle::current(),
            TickScheduler::new(grid),
            table_timers(Some(grid)),
        )
    } else {
        LanePoller::start_with_timers(&Handle::current(), table_timers(None))
    }
}

fn host<F: std::future::Future<Output = ()> + Send + 'static>(poller: &LanePoller, future: F) {
    let _detached = poller.host_uncounted(future).ok().expect("poller runs");
}

#[tokio::test]
async fn a_table_source_puts_the_same_rtp_on_the_wire_as_a_paced_task() {
    let _v2 = override_media_executor_config_on_this_thread(MediaExecutorConfig::v2());
    for (srtp, tick) in [(false, false), (false, true), (true, false), (true, true)] {
        let poller = table_poller(tick);
        let table_call = call(srtp).await;
        let task_call = call(srtp).await;
        let frames = script(25, 0x3c);
        let start = Instant::now() + Duration::from_millis(30);
        let fires = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let source = Box::new(PacedRtp {
            sender: table_call.sender.clone(),
            frames: Arc::clone(&frames),
            cursor: 0,
            start,
            fires: Arc::clone(&fires),
            dropped: Arc::new(AtomicUsize::new(0)),
        });
        let (ended_tx, ended_rx) = oneshot::channel();
        host(&poller, async move {
            let _ = ended_tx.send(supervise(source).await);
        });
        tokio::spawn(paced_task(
            task_call.sender.clone(),
            Arc::clone(&frames),
            start,
        ));
        let from_table = receive(&table_call.peer, frames.len(), srtp).await;
        let from_task = receive(&task_call.peer, frames.len(), srtp).await;
        let ended = ended_rx.await.unwrap();
        assert!(
            matches!(ended, Ended::Done { installs } if installs >= 1),
            "{ended:?} (srtp {srtp}, tick {tick})"
        );
        assert_eq!(
            from_table, from_task,
            "same sequence, timestamps, marker and payloads (srtp {srtp}, tick {tick})"
        );
        for (frame, packet) in from_table.iter().enumerate() {
            assert_eq!(packet.0 as usize, frame, "consecutive sequence numbers");
            assert_eq!(packet.1, timestamp_of(frame));
            assert_eq!(packet.5, frames[frame]);
        }
        // Fired from the table, never before a deadline.
        let fires = fires.lock().clone();
        assert!(!fires.is_empty());
        assert!(fires.iter().all(|(_, deadline, now)| now >= deadline));
        drop(table_call.session);
        drop(task_call.session);
    }
}

/// Many calls on one ticking lane: every frame leaves on its 20 ms cadence
/// from the table (never early, bounded lateness) and the calls add no
/// runtime task, while one paced task per call would.
#[tokio::test]
async fn table_sources_pace_20ms_frames_without_a_task_per_call() {
    const CALLS: usize = 10;
    const FRAMES: usize = 15;
    let _v2 = override_media_executor_config_on_this_thread(MediaExecutorConfig::v2());
    let poller = table_poller(true);
    let mut calls = Vec::new();
    for _ in 0..CALLS {
        calls.push(call(false).await);
    }
    // Let every session's own tasks start before taking the baseline.
    tokio::time::sleep(Duration::from_millis(20)).await;
    let metrics = Handle::current().metrics();
    let baseline = metrics.num_alive_tasks();
    let before = callback_lane_counters();
    let fires = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let start = Instant::now() + Duration::from_millis(30);
    let mut ended = Vec::new();
    for (index, call) in calls.iter().enumerate() {
        let source = Box::new(PacedRtp {
            sender: call.sender.clone(),
            frames: script(FRAMES, index as u8),
            cursor: 0,
            // Spread the calls over the 20 ms frame.
            start: start + Duration::from_micros(1_900 * index as u64),
            fires: Arc::clone(&fires),
            dropped: Arc::new(AtomicUsize::new(0)),
        });
        let (ended_tx, ended_rx) = oneshot::channel();
        ended.push(ended_rx);
        host(&poller, async move {
            let _ = ended_tx.send(supervise(source).await);
        });
    }
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        metrics.num_alive_tasks(),
        baseline,
        "the supervisors are hosted on the poller, not tasks"
    );
    let mut received = Vec::new();
    for call in &calls {
        received.push(receive(&call.peer, FRAMES, false).await);
    }
    for ended_rx in ended {
        assert!(matches!(ended_rx.await.unwrap(), Ended::Done { .. }));
    }
    for (index, wire) in received.iter().enumerate() {
        let timestamps: Vec<u32> = wire.iter().map(|packet| packet.1).collect();
        assert_eq!(
            timestamps,
            (0..FRAMES).map(timestamp_of).collect::<Vec<_>>(),
            "call {index}"
        );
    }
    let fires = fires.lock().clone();
    assert!(fires.len() >= CALLS, "every call fired from the table");
    assert!(
        fires.iter().all(|(_, deadline, now)| now >= deadline),
        "never early"
    );
    let worst = fires
        .iter()
        .map(|(_, deadline, now)| now.saturating_duration_since(*deadline))
        .max()
        .unwrap();
    assert!(
        worst < Duration::from_millis(30),
        "worst lateness {worst:?}"
    );
    // Most frames leave one per fire, on their own deadline.
    let on_time = fires
        .iter()
        .filter(|(_, deadline, now)| now.saturating_duration_since(*deadline) < FRAME)
        .count();
    assert!(
        on_time * 10 >= fires.len() * 9,
        "{on_time} of {}",
        fires.len()
    );
    let after = callback_lane_counters();
    assert!(after.table_lanes >= before.table_lanes);
    // The same calls as paced tasks: one runtime task each.
    for call in &calls {
        tokio::spawn(paced_task(
            call.sender.clone(),
            script(1, 0),
            Instant::now() + Duration::from_millis(500),
        ));
    }
    assert_eq!(metrics.num_alive_tasks(), baseline + CALLS);
}

/// Deregistration and teardown on a real lane: a supervisor that retires
/// its entry on the lane gets the source back at once, a retirement from
/// another thread completes through `finished`, and shutting the lane down
/// drops every source still installed.
#[test]
fn deregistration_and_lane_teardown_release_every_source() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let dropped = Arc::new(AtomicUsize::new(0));
    let (handles_tx, handles_rx) = std::sync::mpsc::channel();
    let (local_tx, local_rx) = std::sync::mpsc::channel();
    let poller = runtime.block_on(async {
        let _v2 = override_media_executor_config_on_this_thread(MediaExecutorConfig::v2());
        let poller = table_poller(true);
        for index in 0..6 {
            let call = call(false).await;
            let source = Box::new(PacedRtp {
                sender: call.sender.clone(),
                frames: script(10_000, index),
                cursor: 0,
                start: Instant::now(),
                fires: Arc::new(parking_lot::Mutex::new(Vec::new())),
                dropped: Arc::clone(&dropped),
            });
            let handles_tx = handles_tx.clone();
            let local_tx = local_tx.clone();
            host(&poller, async move {
                let _call = call;
                let mut handle = install_tx(source, Instant::now())
                    .ok()
                    .expect("a table on this poller");
                match index {
                    // Retired by its supervisor on the lane, after a few
                    // frames: synchronous, the source comes back now.
                    0 | 1 => {
                        crate::media_clock::sleep(Duration::from_millis(60)).await;
                        let back = handle.retire(RetireReason::Control);
                        let _ = local_tx.send(matches!(back, Retired::Now(_)));
                    }
                    // Handed to another thread, which retires it (the call
                    // stays up meanwhile).
                    2 => {
                        let _ = handles_tx.send(handle);
                        std::future::pending::<()>().await;
                    }
                    // Still installed when the lane goes.
                    _ => {
                        std::future::pending::<()>().await;
                    }
                }
            });
        }
        poller
    });
    let lane = std::thread::spawn(move || {
        runtime.block_on(async { tokio::time::sleep(Duration::from_millis(400)).await });
        drop(poller);
        drop(runtime);
    });
    let mut handle = handles_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("handed over");
    std::thread::sleep(Duration::from_millis(100));
    assert!(matches!(
        handle.retire(RetireReason::Control),
        Retired::Pending
    ));
    let back = futures::executor::block_on(handle.finished());
    assert!(matches!(back, Some(LaneTxReturn::Retired(_))), "{back:?}");
    drop(back);
    for _ in 0..2 {
        assert!(local_rx.recv_timeout(Duration::from_secs(5)).unwrap());
    }
    lane.join().unwrap();
    assert_eq!(
        dropped.load(Ordering::Acquire),
        6,
        "every source was dropped: retired ones by their supervisors, the rest with the lane"
    );
}

/// Under v1 nothing changes: a poller started now keeps no table,
/// `spawn_on_media_lane` runs the supervisor as a task, `install_tx` gives
/// the source back and the supervisor's paced loop sends every frame.
#[tokio::test]
async fn under_v1_there_is_no_table_and_the_source_runs_as_a_paced_loop() {
    let _v1 = override_media_executor_config_on_this_thread(MediaExecutorConfig::v1());
    assert!(!PollerTimers::configured(None).callback_table);
    assert!(!callback_send_enabled());
    let call = call(false).await;
    let before = callback_lane_counters().install_fallbacks;
    let frames = script(5, 0x11);
    let source = Box::new(PacedRtp {
        sender: call.sender.clone(),
        frames: Arc::clone(&frames),
        cursor: 0,
        start: Instant::now(),
        fires: Arc::new(parking_lot::Mutex::new(Vec::new())),
        dropped: Arc::new(AtomicUsize::new(0)),
    });
    let (ended_tx, ended_rx) = oneshot::channel();
    crate::executor::spawn_on_media_lane(&Handle::current(), async move {
        let _ = ended_tx.send(supervise(source).await);
    });
    let wire = receive(&call.peer, frames.len(), false).await;
    assert_eq!(ended_rx.await.unwrap(), Ended::Done { installs: 0 });
    assert!(callback_lane_counters().install_fallbacks > before);
    assert_eq!(
        wire.iter()
            .map(|packet| packet.5.clone())
            .collect::<Vec<_>>(),
        *frames
    );
}
