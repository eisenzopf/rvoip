//! Bounded, one-source-to-many real-time media routing.
//!
//! A `MediaStream::frames_in()` receiver is intentionally single-take. The
//! media graph owns that receiver once and exposes dynamic sink routes so a
//! call peer, recorder, UCTP publisher, and MOQT publisher can observe the
//! same source without racing for frames.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock as StdRwLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use rvoip_media_core::codec::audio::{AudioCodec, OpusApplication, OpusCodec, OpusConfig};
use rvoip_media_core::codec::factory::CodecFactory;
use rvoip_media_core::error::CodecError;
use rvoip_media_core::processing::format::{ConversionParams, FormatConverter};
use rvoip_media_core::types::SampleRate;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot, watch, Notify};
use tokio::task::AbortHandle;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::bridge::{codec_to_pt, frame_pump::DEFAULT_TELEPHONE_EVENT_PT};
use crate::capability::CodecInfo;
use crate::error::{Result, RvoipError};
use crate::ids::MediaRouteId;
use crate::stream::MediaFrame;

const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(1);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const SNAPSHOT_PUBLISH_INTERVAL: Duration = Duration::from_secs(1);
/// Maximum publication cadence for retained source-activity observations.
pub const MEDIA_GRAPH_ACTIVITY_OBSERVATION_INTERVAL: Duration = Duration::from_secs(1);
const RECENT_EVICTION_LIMIT: usize = 64;
const CONTROL_QUEUE_CAPACITY: usize = 256;
const SINK_EVENT_QUEUE_CAPACITY: usize = 256;

/// Default graph fanout ceiling. This admits the 1,000-listener direct UCTP
/// target while retaining 24 routes for the call peer, recorders, and other
/// operational observers.
pub const DEFAULT_MEDIA_GRAPH_MAX_SINKS: usize = 1_024;

/// Stable identifier for the lifetime of a media graph.
///
/// It is intentionally defined here rather than in the shared ID vocabulary:
/// a graph is an rvoip-core runtime concern, not a cross-adapter wire type.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct MediaGraphId(String);

impl MediaGraphId {
    pub fn new() -> Self {
        Self(format!("graph_{}", Uuid::new_v4().simple()))
    }

    pub fn from_string(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for MediaGraphId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for MediaGraphId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug)]
pub struct MediaGraphPolicy {
    /// Maximum installed or queued sink routes. Admission is reserved before
    /// an add command enters the bounded control queue, so concurrent callers
    /// cannot oversubscribe the graph.
    pub max_sinks: usize,
    pub sink_queue_frames: usize,
    /// Frames retained before the first sink is registered. The buffer is
    /// always bounded and drops its oldest frame on overflow.
    pub pre_sink_buffer_frames: usize,
    pub eviction_window: Duration,
    pub eviction_drop_ratio: f32,
    pub minimum_eviction_samples: usize,
}

impl Default for MediaGraphPolicy {
    fn default() -> Self {
        Self {
            max_sinks: DEFAULT_MEDIA_GRAPH_MAX_SINKS,
            sink_queue_frames: 10,
            pre_sink_buffer_frames: 10,
            eviction_window: Duration::from_secs(10),
            eviction_drop_ratio: 0.25,
            minimum_eviction_samples: 50,
        }
    }
}

/// Latest coalesced proof that the graph consumed media from its single
/// authoritative source receiver.
///
/// `source_frames` is diagnostic only. Consumers that persist activity should
/// assign their own consecutive delivery generation because this retained
/// value can skip counts while they are backpressured.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaGraphActivityObservation {
    pub source_frames: u64,
    pub observed_at: DateTime<Utc>,
}

/// Current state of the graph's single source receiver.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaGraphSourceState {
    Open,
    Closed,
    Shutdown,
    Aborted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaGraphEvictionReason {
    SlowConsumer,
}

/// Why a managed media route reached its terminal state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaGraphRouteTerminalReason {
    OwnerRemoved,
    TargetClosed,
    SlowConsumerEvicted,
    GraphShutdown,
    SourceClosed,
    GraphAborted,
}

/// Latest lifecycle state of a managed media route.
///
/// This is delivered through a Tokio watch channel, so observers retain one
/// bounded latest value rather than an unbounded event backlog.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaGraphRouteState {
    Pending,
    Active,
    Terminal(MediaGraphRouteTerminalReason),
}

/// Cloneable lifecycle observer for one graph sink route.
///
/// Status observers do not own graph membership. Cloning this value never
/// extends the route's lifetime after its [`ManagedMediaRoute`] owner drops.
#[derive(Clone)]
pub struct MediaGraphRouteStatus {
    route_id: MediaRouteId,
    state: watch::Receiver<MediaGraphRouteState>,
}

impl MediaGraphRouteStatus {
    pub fn id(&self) -> &MediaRouteId {
        &self.route_id
    }

    pub fn state(&self) -> MediaGraphRouteState {
        let state = *self.state.borrow();
        if matches!(state, MediaGraphRouteState::Terminal(_)) {
            return state;
        }
        let receiver = self.state.clone();
        if receiver.has_changed().is_err() {
            MediaGraphRouteState::Terminal(MediaGraphRouteTerminalReason::GraphAborted)
        } else {
            state
        }
    }

    /// Wait until the actor has installed the route. If the graph terminates
    /// first, return the retained terminal reason instead of hanging.
    pub async fn wait_active(&self) -> std::result::Result<(), MediaGraphRouteTerminalReason> {
        let mut receiver = self.state.clone();
        loop {
            match *receiver.borrow_and_update() {
                MediaGraphRouteState::Active => return Ok(()),
                MediaGraphRouteState::Terminal(reason) => return Err(reason),
                MediaGraphRouteState::Pending => {}
            }
            if receiver.changed().await.is_err() {
                return Err(MediaGraphRouteTerminalReason::GraphAborted);
            }
        }
    }

    pub async fn wait_terminal(&self) -> MediaGraphRouteTerminalReason {
        let mut receiver = self.state.clone();
        loop {
            if let MediaGraphRouteState::Terminal(reason) = *receiver.borrow_and_update() {
                return reason;
            }
            if receiver.changed().await.is_err() {
                return MediaGraphRouteTerminalReason::GraphAborted;
            }
        }
    }
}

/// Owning lease for a managed graph route.
///
/// Dropping this lease signals owner cancellation and best-effort queues a
/// fast removal command. The actor also observes cancellation during bounded
/// frame/periodic maintenance, so queue saturation cannot retain the route.
/// Callers may clone [`Self::status`] freely without keeping the route alive.
#[must_use = "dropping the managed route immediately removes its sink"]
pub struct ManagedMediaRoute {
    status: MediaGraphRouteStatus,
    commands: mpsc::Sender<Command>,
    owner_liveness: Arc<RouteOwnerLiveness>,
    remove_on_drop: bool,
}

impl ManagedMediaRoute {
    pub fn id(&self) -> &MediaRouteId {
        self.status.id()
    }

    pub fn status(&self) -> MediaGraphRouteStatus {
        self.status.clone()
    }

    pub fn state(&self) -> MediaGraphRouteState {
        self.status.state()
    }

    pub async fn wait_active(&self) -> std::result::Result<(), MediaGraphRouteTerminalReason> {
        self.status.wait_active().await
    }

    pub async fn wait_terminal(&self) -> MediaGraphRouteTerminalReason {
        self.status.wait_terminal().await
    }

    /// Remove this route with actor acknowledgement. Clone [`Self::status`]
    /// first when the caller also wants to observe the retained terminal
    /// reason after consuming the owning lease.
    pub async fn remove(mut self) -> Result<bool> {
        let (ack, done) = oneshot::channel();
        tokio::time::timeout(
            SNAPSHOT_TIMEOUT,
            self.commands.send(Command::Remove {
                route_id: self.status.route_id.clone(),
                ack: Some(ack),
            }),
        )
        .await
        .map_err(|_| RvoipError::InvalidState("media graph command queue is full"))?
        .map_err(|_| RvoipError::InvalidState("media graph is closed"))?;
        self.remove_on_drop = false;
        tokio::time::timeout(SNAPSHOT_TIMEOUT, done)
            .await
            .map_err(|_| RvoipError::InvalidState("media graph removal timed out"))?
            .map_err(|_| RvoipError::InvalidState("media graph removal was cancelled"))
    }

    fn into_unmanaged_route_id(mut self) -> MediaRouteId {
        self.remove_on_drop = false;
        self.status.route_id.clone()
    }
}

impl Drop for ManagedMediaRoute {
    fn drop(&mut self) {
        if self.remove_on_drop {
            // The bounded control queue is only a fast path. The actor also
            // observes this signal during frame processing and its periodic
            // maintenance tick, so a full queue cannot retain an orphan.
            self.owner_liveness.cancel();
            let _ = self.commands.try_send(Command::Remove {
                route_id: self.status.route_id.clone(),
                ack: None,
            });
        }
    }
}

/// Last-known state for an active sink route.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaGraphSinkSnapshot {
    pub route_id: MediaRouteId,
    pub target_codec: CodecInfo,
    pub target_payload_type: u8,
    pub queue_depth: usize,
    pub queue_capacity: usize,
    pub offered_frames: u64,
    pub dropped_frames: u64,
    pub rolling_samples: usize,
    pub rolling_drops: usize,
    pub rolling_drop_ratio: f32,
}

/// Codec-group state. A source frame is transcoded at most once per group and
/// the resulting immutable payload is cloned cheaply into every member sink.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MediaGraphCodecGroupSnapshot {
    pub target_codec: CodecInfo,
    pub target_payload_type: u8,
    pub sink_routes: Vec<MediaRouteId>,
    pub transcoding: bool,
    pub source_frames_routed: u64,
    pub transcode_operations: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MediaGraphEvictionSnapshot {
    pub route_id: MediaRouteId,
    pub reason: MediaGraphEvictionReason,
    pub offered_frames: u64,
    pub dropped_frames: u64,
}

/// Point-in-time operational view of a graph.
///
/// The leading concurrent `MediaGraphHandle::snapshot` request places a
/// barrier on the graph command queue; concurrent followers coalesce onto the
/// retained view. `latest_snapshot_arc` is a cheap non-blocking last-known view
/// and remains available after the graph has stopped.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaGraphSnapshot {
    pub graph_id: MediaGraphId,
    pub source_state: MediaGraphSourceState,
    pub source_codec: CodecInfo,
    pub source_payload_type: u8,
    pub source_frames: u64,
    pub sink_offers: u64,
    pub dropped_frames: u64,
    pub evictions: u64,
    pub transcode_operations: u64,
    pub transcode_errors: u64,
    pub sinks: Vec<MediaGraphSinkSnapshot>,
    pub codec_groups: Vec<MediaGraphCodecGroupSnapshot>,
    pub recent_evictions: Vec<MediaGraphEvictionSnapshot>,
}

enum Command {
    Add {
        route_id: MediaRouteId,
        codec: CodecInfo,
        target: mpsc::Sender<MediaFrame>,
        owner_liveness: Arc<RouteOwnerLiveness>,
        admission: SinkAdmissionPermit,
    },
    Remove {
        route_id: MediaRouteId,
        ack: Option<oneshot::Sender<bool>>,
    },
    UpdateSourceCodec {
        codec: CodecInfo,
        source_pt: u8,
        ack: oneshot::Sender<Result<()>>,
    },
    UpdateSinkCodec {
        route_id: MediaRouteId,
        codec: CodecInfo,
        target_pt: u8,
        ack: oneshot::Sender<Result<()>>,
    },
    /// Compatibility command for the original payload-type based API.
    UpdateRoute {
        route_id: MediaRouteId,
        source_codec: CodecInfo,
        source_pt: u8,
        target_codec: CodecInfo,
        target_pt: u8,
        ack: oneshot::Sender<Result<()>>,
    },
    Snapshot(oneshot::Sender<Arc<MediaGraphSnapshot>>),
    Shutdown,
}

type RetainedSnapshot = Arc<StdRwLock<Arc<MediaGraphSnapshot>>>;
type SinkTaskRegistry = Arc<Mutex<Vec<AbortHandle>>>;
type RouteStatusRegistry = Arc<Mutex<HashMap<MediaRouteId, watch::Sender<MediaGraphRouteState>>>>;

#[derive(Default)]
struct RouteOwnerLiveness {
    cancelled: AtomicBool,
}

impl RouteOwnerLiveness {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

struct SinkAdmissionState {
    in_use: AtomicUsize,
    maximum: usize,
}

impl SinkAdmissionState {
    fn new(maximum: usize) -> Self {
        Self {
            in_use: AtomicUsize::new(0),
            maximum,
        }
    }

    fn try_acquire(self: &Arc<Self>) -> Option<SinkAdmissionPermit> {
        let mut in_use = self.in_use.load(Ordering::Acquire);
        loop {
            if in_use >= self.maximum {
                return None;
            }
            match self.in_use.compare_exchange_weak(
                in_use,
                in_use + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Some(SinkAdmissionPermit {
                        state: Arc::clone(self),
                    });
                }
                Err(observed) => in_use = observed,
            }
        }
    }
}

struct SinkAdmissionPermit {
    state: Arc<SinkAdmissionState>,
}

impl Drop for SinkAdmissionPermit {
    fn drop(&mut self) {
        let previous = self.state.in_use.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "media graph sink admission underflow");
    }
}

#[derive(Clone)]
pub struct MediaGraphHandle {
    graph_id: MediaGraphId,
    commands: mpsc::Sender<Command>,
    abort: AbortHandle,
    latest_snapshot: RetainedSnapshot,
    snapshot_in_flight: Arc<AtomicBool>,
    completion: watch::Receiver<Option<MediaGraphSourceState>>,
    activity: watch::Receiver<Option<MediaGraphActivityObservation>>,
    route_statuses: RouteStatusRegistry,
    sink_admission: Arc<SinkAdmissionState>,
}

impl MediaGraphHandle {
    pub fn id(&self) -> &MediaGraphId {
        &self.graph_id
    }

    /// Subscribe to a bounded retained source-activity observation.
    ///
    /// The graph publishes at most one value per configured observation
    /// interval and overwrites an unread value with the newest one. A slow
    /// observer therefore cannot stall or grow memory in the media path.
    pub fn subscribe_activity(&self) -> watch::Receiver<Option<MediaGraphActivityObservation>> {
        self.activity.clone()
    }

    pub fn add_sink(
        &self,
        codec: CodecInfo,
        target: mpsc::Sender<MediaFrame>,
    ) -> Result<MediaRouteId> {
        self.add_managed_sink(codec, target)
            .map(ManagedMediaRoute::into_unmanaged_route_id)
    }

    /// Add a sink and retain a bounded, cloneable lifecycle observer for it.
    /// The legacy [`Self::add_sink`] API remains a route-ID-only wrapper.
    pub fn add_managed_sink(
        &self,
        codec: CodecInfo,
        target: mpsc::Sender<MediaFrame>,
    ) -> Result<ManagedMediaRoute> {
        payload_type_for_codec(&codec)
            .ok_or_else(|| RvoipError::UnsupportedCodec(codec.name.clone()))?;
        let Some(admission) = self.sink_admission.try_acquire() else {
            metrics::counter!(
                "rvoip_media_graph_sink_admission_rejections_total",
                "reason" => "max-sinks"
            )
            .increment(1);
            return Err(RvoipError::AdmissionRejected(
                "media graph maximum sink count reached",
            ));
        };
        let route_id = MediaRouteId::new();
        let (status_tx, status_rx) = watch::channel(MediaGraphRouteState::Pending);
        let owner_liveness = Arc::new(RouteOwnerLiveness::default());
        self.route_statuses
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(route_id.clone(), status_tx);
        if let Err(error) = self.commands.try_send(Command::Add {
            route_id: route_id.clone(),
            codec,
            target,
            owner_liveness: Arc::clone(&owner_liveness),
            admission,
        }) {
            self.route_statuses
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&route_id);
            return Err(map_try_send_error(error));
        }
        Ok(ManagedMediaRoute {
            status: MediaGraphRouteStatus {
                route_id,
                state: status_rx,
            },
            commands: self.commands.clone(),
            owner_liveness,
            remove_on_drop: true,
        })
    }

    /// Queue a removal command. The return value reports whether the command
    /// was accepted, not whether the route existed. Use
    /// `remove_sink_and_wait` when route-existence acknowledgement matters.
    pub fn remove_sink(&self, route_id: MediaRouteId) -> bool {
        self.commands
            .try_send(Command::Remove {
                route_id,
                ack: None,
            })
            .is_ok()
    }

    pub async fn remove_sink_and_wait(&self, route_id: MediaRouteId) -> Result<bool> {
        let (ack, done) = oneshot::channel();
        self.send_control(Command::Remove {
            route_id,
            ack: Some(ack),
        })
        .await?;
        tokio::time::timeout(SNAPSHOT_TIMEOUT, done)
            .await
            .map_err(|_| RvoipError::InvalidState("media graph removal timed out"))?
            .map_err(|_| RvoipError::InvalidState("media graph removal was cancelled"))
    }

    /// Update the source codec and rebuild every codec group's transcoder.
    pub async fn update_source_codec(&self, codec: CodecInfo) -> Result<()> {
        let source_pt = payload_type_for_codec(&codec)
            .ok_or_else(|| RvoipError::UnsupportedCodec(codec.name.clone()))?;
        let (ack, done) = oneshot::channel();
        self.send_control(Command::UpdateSourceCodec {
            codec,
            source_pt,
            ack,
        })
        .await?;
        await_update(done).await
    }

    /// Move one sink to the codec group represented by `codec`.
    pub async fn update_sink_codec(&self, route_id: MediaRouteId, codec: CodecInfo) -> Result<()> {
        let target_pt = payload_type_for_codec(&codec)
            .ok_or_else(|| RvoipError::UnsupportedCodec(codec.name.clone()))?;
        let (ack, done) = oneshot::channel();
        self.send_control(Command::UpdateSinkCodec {
            route_id,
            codec,
            target_pt,
            ack,
        })
        .await?;
        await_update(done).await
    }

    /// Compatibility wrapper for callers that still renegotiate with RTP
    /// payload types. New code should call `update_source_codec` and
    /// `update_sink_codec` independently.
    pub async fn update_route(
        &self,
        route_id: MediaRouteId,
        source_pt: u8,
        target_pt: u8,
    ) -> Result<()> {
        let source_codec = codec_for_payload_type(source_pt)
            .ok_or_else(|| RvoipError::UnsupportedCodec(format!("rtp-payload-type-{source_pt}")))?;
        let target_codec = codec_for_payload_type(target_pt)
            .ok_or_else(|| RvoipError::UnsupportedCodec(format!("rtp-payload-type-{target_pt}")))?;
        let (ack, done) = oneshot::channel();
        self.send_control(Command::UpdateRoute {
            route_id,
            source_codec,
            source_pt,
            target_codec,
            target_pt,
            ack,
        })
        .await?;
        await_update(done).await
    }

    /// Return a command-barrier-consistent snapshot when this call leads a
    /// snapshot batch, a coalesced retained snapshot when another request is
    /// already pending, or the final retained snapshot after shutdown.
    pub async fn snapshot(&self) -> MediaGraphSnapshot {
        (*self.snapshot_arc().await).clone()
    }

    /// Arc-returning snapshot API for high-frequency diagnostics. Concurrent
    /// callers coalesce behind at most one actor request; followers receive the
    /// retained snapshot instead of growing the control queue.
    pub async fn snapshot_arc(&self) -> Arc<MediaGraphSnapshot> {
        if self
            .snapshot_in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return self.latest_snapshot_arc();
        }
        let _request_guard = SnapshotRequestGuard(Arc::clone(&self.snapshot_in_flight));
        let (send, receive) = oneshot::channel();
        if self.commands.try_send(Command::Snapshot(send)).is_ok() {
            if let Ok(Ok(snapshot)) = tokio::time::timeout(SNAPSHOT_TIMEOUT, receive).await {
                return snapshot;
            }
        }
        self.latest_snapshot_arc()
    }

    /// Return the most recently published snapshot without waiting on the
    /// graph actor.
    pub fn latest_snapshot(&self) -> MediaGraphSnapshot {
        (*self.latest_snapshot_arc()).clone()
    }

    /// Cheap retained-snapshot access. The read lock is held only long enough
    /// to clone an Arc; callers never contend while inspecting its contents.
    pub fn latest_snapshot_arc(&self) -> Arc<MediaGraphSnapshot> {
        read_snapshot(&self.latest_snapshot)
    }

    pub fn shutdown(&self) {
        if matches!(
            self.commands.try_send(Command::Shutdown),
            Err(mpsc::error::TrySendError::Full(_))
        ) {
            // A saturated control plane must not make shutdown impossible.
            self.abort.abort();
        }
    }

    /// Request graceful shutdown and wait for both the graph actor and every
    /// sink-forwarding task to converge on a terminal state.
    pub async fn shutdown_and_wait(&self) -> Result<MediaGraphSourceState> {
        self.shutdown();
        self.wait_closed().await
    }

    /// Wait for graph and sink-task convergence without initiating shutdown.
    pub async fn wait_closed(&self) -> Result<MediaGraphSourceState> {
        let mut completion = self.completion.clone();
        let actor = self.abort.clone();
        tokio::time::timeout(SHUTDOWN_TIMEOUT, async move {
            let state = loop {
                if let Some(state) = *completion.borrow() {
                    break state;
                }
                completion
                    .changed()
                    .await
                    .map_err(|_| RvoipError::InvalidState("media graph completion was dropped"))?;
            };
            while !actor.is_finished() {
                tokio::task::yield_now().await;
            }
            Ok(state)
        })
        .await
        .map_err(|_| RvoipError::InvalidState("media graph shutdown timed out"))?
    }

    pub fn abort_handle(&self) -> AbortHandle {
        self.abort.clone()
    }

    async fn send_control(&self, command: Command) -> Result<()> {
        tokio::time::timeout(SNAPSHOT_TIMEOUT, self.commands.send(command))
            .await
            .map_err(|_| RvoipError::InvalidState("media graph command queue is full"))?
            .map_err(|_| RvoipError::InvalidState("media graph is closed"))
    }
}

struct SnapshotRequestGuard(Arc<AtomicBool>);

impl Drop for SnapshotRequestGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn map_try_send_error(error: mpsc::error::TrySendError<Command>) -> RvoipError {
    match error {
        mpsc::error::TrySendError::Full(_) => {
            RvoipError::InvalidState("media graph command queue is full")
        }
        mpsc::error::TrySendError::Closed(_) => RvoipError::InvalidState("media graph is closed"),
    }
}

fn payload_type_for_codec(codec: &CodecInfo) -> Option<u8> {
    codec_to_pt(codec.name.trim())
}

/// Validate codec identity before transferring ownership of a stream's
/// single-consumer receiver into a graph.
pub fn validate_media_graph_codec(codec: &CodecInfo) -> Result<()> {
    payload_type_for_codec(codec)
        .map(|_| ())
        .ok_or_else(|| RvoipError::UnsupportedCodec(codec.name.clone()))
}

async fn await_update(done: oneshot::Receiver<Result<()>>) -> Result<()> {
    tokio::time::timeout(SNAPSHOT_TIMEOUT, done)
        .await
        .map_err(|_| RvoipError::InvalidState("media graph update timed out"))?
        .map_err(|_| RvoipError::InvalidState("media graph update was cancelled"))?
}

struct SinkQueueState {
    frames: VecDeque<MediaFrame>,
    closed: bool,
}

struct SinkQueue {
    capacity: usize,
    state: Mutex<SinkQueueState>,
    notify: Notify,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OfferResult {
    Enqueued,
    DroppedOldest,
    Closed,
}

impl SinkQueue {
    fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            capacity,
            state: Mutex::new(SinkQueueState {
                frames: VecDeque::with_capacity(capacity),
                closed: false,
            }),
            notify: Notify::new(),
        }
    }

    /// Enqueue without awaiting a slow sink. The oldest queued frame is
    /// discarded when full so the sink always sees the freshest media.
    fn offer(&self, frame: MediaFrame) -> OfferResult {
        let result = {
            let mut state = self.state.lock().expect("media sink queue poisoned");
            if state.closed {
                return OfferResult::Closed;
            }
            let result = if state.frames.len() >= self.capacity {
                state.frames.pop_front();
                OfferResult::DroppedOldest
            } else {
                OfferResult::Enqueued
            };
            state.frames.push_back(frame);
            result
        };
        self.notify.notify_one();
        result
    }

    async fn receive(&self) -> Option<MediaFrame> {
        loop {
            let closed = {
                let mut state = self.state.lock().expect("media sink queue poisoned");
                if let Some(frame) = state.frames.pop_front() {
                    return Some(frame);
                }
                state.closed
            };
            if closed {
                return None;
            }
            self.notify.notified().await;
        }
    }

    fn depth(&self) -> usize {
        self.state
            .lock()
            .expect("media sink queue poisoned")
            .frames
            .len()
    }

    fn close(&self) {
        {
            let mut state = self.state.lock().expect("media sink queue poisoned");
            state.closed = true;
            state.frames.clear();
        }
        self.notify.notify_waiters();
    }
}

struct SinkRuntime {
    target_codec: CodecInfo,
    target_pt: u8,
    group_key: CodecGroupKey,
    owner_liveness: Arc<RouteOwnerLiveness>,
    /// Releasing the runtime route returns one slot to the graph-wide
    /// admission budget. Pending add commands hold the same kind of permit.
    _admission: SinkAdmissionPermit,
    /// RTP clock ownership is per route, not per codec group. This lets a
    /// route retain its timestamp epoch when fmtp/payload changes move it
    /// between groups while payload transcoding remains shared by the group.
    clock: RtpClockTranslator,
    queue: Arc<SinkQueue>,
    task: AbortHandle,
    history: VecDeque<(Instant, bool)>,
    rolling_drops: usize,
    offered_frames: u64,
    dropped_frames: u64,
}

impl SinkRuntime {
    fn record_offer(&mut self, now: Instant, dropped: bool, policy: &MediaGraphPolicy) -> bool {
        self.offered_frames = self.offered_frames.saturating_add(1);
        if dropped {
            self.dropped_frames = self.dropped_frames.saturating_add(1);
            self.rolling_drops = self.rolling_drops.saturating_add(1);
        }
        self.history.push_back((now, dropped));
        while self
            .history
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) > policy.eviction_window)
        {
            if self.history.pop_front().is_some_and(|(_, dropped)| dropped) {
                self.rolling_drops = self.rolling_drops.saturating_sub(1);
            }
        }
        if self.history.len() < policy.minimum_eviction_samples {
            return false;
        }
        self.rolling_drops as f32 / self.history.len() as f32 > policy.eviction_drop_ratio
    }

    fn rolling_drop_counts(&self) -> (usize, usize) {
        (self.history.len(), self.rolling_drops)
    }
}

impl Drop for SinkRuntime {
    fn drop(&mut self) {
        self.queue.close();
        self.task.abort();
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct CodecGroupKey {
    payload_type: u8,
    name: String,
    clock_rate_hz: u32,
    channels: u8,
    fmtp: Option<String>,
}

impl CodecGroupKey {
    fn new(codec: &CodecInfo, payload_type: u8) -> Self {
        Self {
            payload_type,
            name: canonical_codec_name(codec, payload_type),
            clock_rate_hz: codec.clock_rate_hz,
            channels: codec.channels,
            fmtp: normalize_fmtp(codec.fmtp.as_deref()),
        }
    }
}

fn canonical_codec_name(codec: &CodecInfo, payload_type: u8) -> String {
    match payload_type {
        0 => "pcmu".into(),
        8 => "pcma".into(),
        18 => "g729".into(),
        111 => "opus".into(),
        _ => codec.name.trim().to_ascii_lowercase(),
    }
}

fn normalize_fmtp(fmtp: Option<&str>) -> Option<String> {
    let mut parameters: Vec<_> = fmtp
        .unwrap_or_default()
        .split(';')
        .filter_map(|parameter| {
            let parameter = parameter.trim();
            if parameter.is_empty() {
                return None;
            }
            let normalized = match parameter.split_once('=') {
                Some((name, value)) => {
                    format!("{}={}", name.trim().to_ascii_lowercase(), value.trim())
                }
                None => parameter.to_ascii_lowercase(),
            };
            Some(normalized)
        })
        .collect();
    parameters.sort();
    (!parameters.is_empty()).then(|| parameters.join(";"))
}

struct RtpClockTranslator {
    source_rate: u32,
    target_rate: u32,
    last_source: Option<u32>,
    last_target: u32,
    remainder: u64,
}

impl RtpClockTranslator {
    fn new(source_rate: u32, target_rate: u32) -> Self {
        Self {
            source_rate: source_rate.max(1),
            target_rate: target_rate.max(1),
            last_source: None,
            last_target: 0,
            remainder: 0,
        }
    }

    fn translate(&mut self, source_timestamp: u32) -> u32 {
        let Some(last_source) = self.last_source.replace(source_timestamp) else {
            self.last_target = source_timestamp;
            return source_timestamp;
        };
        let source_delta = source_timestamp.wrapping_sub(last_source) as u64;
        let numerator = source_delta
            .saturating_mul(self.target_rate as u64)
            .saturating_add(self.remainder);
        let target_delta = numerator / self.source_rate as u64;
        self.remainder = numerator % self.source_rate as u64;
        self.last_target = self.last_target.wrapping_add(target_delta as u32);
        self.last_target
    }

    fn reconfigure(&mut self, source_rate: u32, target_rate: u32) {
        self.source_rate = source_rate.max(1);
        self.target_rate = target_rate.max(1);
        self.remainder = 0;
    }
}

struct ConfiguredTranscodingSession {
    source_codec: Box<dyn AudioCodec>,
    target_codec: Box<dyn AudioCodec>,
    format_converter: FormatConverter,
}

impl ConfiguredTranscodingSession {
    fn new(
        source: &CodecInfo,
        source_pt: u8,
        target: &CodecInfo,
        target_pt: u8,
    ) -> rvoip_media_core::Result<Self> {
        Ok(Self {
            source_codec: create_configured_codec(source, source_pt)?,
            target_codec: create_configured_codec(target, target_pt)?,
            format_converter: FormatConverter::new(),
        })
    }

    fn transcode(&mut self, encoded_data: &[u8]) -> rvoip_media_core::Result<Vec<u8>> {
        let source_frame = self.source_codec.decode(encoded_data)?;
        let target_info = self.target_codec.get_info();
        let converted = if source_frame.sample_rate != target_info.sample_rate
            || source_frame.channels != target_info.channels
        {
            let target_rate = SampleRate::from_hz(target_info.sample_rate).ok_or_else(|| {
                CodecError::InvalidParameters {
                    details: format!("unsupported target clock rate {}", target_info.sample_rate),
                }
            })?;
            self.format_converter
                .convert_frame(
                    &source_frame,
                    &ConversionParams::new(target_rate, target_info.channels),
                )?
                .frame
        } else {
            source_frame
        };
        self.target_codec.encode(&converted)
    }
}

struct ConfiguredTranscoder {
    source_codec: CodecInfo,
    source_pt: u8,
    target_codec: CodecInfo,
    target_pt: u8,
    session: Option<ConfiguredTranscodingSession>,
}

impl ConfiguredTranscoder {
    fn new(source_codec: CodecInfo, source_pt: u8, target_codec: CodecInfo, target_pt: u8) -> Self {
        Self {
            source_codec,
            source_pt,
            target_codec,
            target_pt,
            session: None,
        }
    }

    fn transcode(&mut self, payload: &[u8]) -> rvoip_media_core::Result<Vec<u8>> {
        if self.session.is_none() {
            self.session = Some(ConfiguredTranscodingSession::new(
                &self.source_codec,
                self.source_pt,
                &self.target_codec,
                self.target_pt,
            )?);
        }
        self.session
            .as_mut()
            .expect("configured transcoder session initialized")
            .transcode(payload)
    }
}

fn create_configured_codec(
    codec: &CodecInfo,
    payload_type: u8,
) -> rvoip_media_core::Result<Box<dyn AudioCodec>> {
    match payload_type {
        0 | 8 | 18 => CodecFactory::create_codec(
            payload_type,
            Some(codec.clock_rate_hz),
            Some(codec.channels.into()),
        ),
        111 => {
            let sample_rate = SampleRate::from_hz(codec.clock_rate_hz).ok_or_else(|| {
                CodecError::InvalidParameters {
                    details: format!("unsupported Opus clock rate {}", codec.clock_rate_hz),
                }
            })?;
            let mut config = OpusConfig {
                application: OpusApplication::Voip,
                ..OpusConfig::default()
            };
            for parameter in normalize_fmtp(codec.fmtp.as_deref())
                .as_deref()
                .unwrap_or_default()
                .split(';')
            {
                if let Some(("maxaveragebitrate", value)) = parameter.split_once('=') {
                    if let Ok(bitrate) = value.parse::<u32>() {
                        if (6_000..=510_000).contains(&bitrate) {
                            config.bitrate = bitrate;
                        }
                    }
                }
                if parameter == "cbr=1" {
                    config.vbr = false;
                }
            }
            Ok(Box::new(OpusCodec::new(
                sample_rate,
                codec.channels,
                config,
            )?))
        }
        _ => Err(CodecError::UnsupportedPayloadType { payload_type }.into()),
    }
}

struct CodecGroup {
    target_codec: CodecInfo,
    target_pt: u8,
    transcoder: Option<ConfiguredTranscoder>,
    sinks: HashSet<MediaRouteId>,
    source_frames_routed: u64,
    transcode_operations: u64,
}

impl CodecGroup {
    fn new(
        source_codec: &CodecInfo,
        source_pt: u8,
        target_codec: CodecInfo,
        target_pt: u8,
    ) -> Self {
        Self {
            transcoder: make_transcoder(source_codec, source_pt, &target_codec, target_pt),
            target_codec,
            target_pt,
            sinks: HashSet::new(),
            source_frames_routed: 0,
            transcode_operations: 0,
        }
    }
}

fn make_transcoder(
    source_codec: &CodecInfo,
    source_pt: u8,
    target_codec: &CodecInfo,
    target_pt: u8,
) -> Option<ConfiguredTranscoder> {
    (CodecGroupKey::new(source_codec, source_pt) != CodecGroupKey::new(target_codec, target_pt))
        .then(|| {
            ConfiguredTranscoder::new(
                source_codec.clone(),
                source_pt,
                target_codec.clone(),
                target_pt,
            )
        })
}

#[derive(Default)]
struct GraphStats {
    source_frames: u64,
    sink_offers: u64,
    dropped_frames: u64,
    evictions: u64,
    transcode_operations: u64,
    transcode_errors: u64,
    recent_evictions: VecDeque<MediaGraphEvictionSnapshot>,
}

impl GraphStats {
    fn record_eviction(&mut self, route_id: MediaRouteId, sink: &SinkRuntime) {
        self.evictions = self.evictions.saturating_add(1);
        if self.recent_evictions.len() == RECENT_EVICTION_LIMIT {
            self.recent_evictions.pop_front();
        }
        self.recent_evictions.push_back(MediaGraphEvictionSnapshot {
            route_id,
            reason: MediaGraphEvictionReason::SlowConsumer,
            offered_frames: sink.offered_frames,
            dropped_frames: sink.dropped_frames,
        });
    }
}

/// Balances process-wide gauges by delta. A graph must never `set` a gauge to
/// its local count because doing so corrupts the aggregate when graphs overlap.
struct AggregateMetricsGuard {
    sinks: usize,
    codec_groups: usize,
}

impl AggregateMetricsGuard {
    fn new() -> Self {
        metrics::gauge!("rvoip_media_graphs_active").increment(1.0);
        Self {
            sinks: 0,
            codec_groups: 0,
        }
    }

    fn set_sink_count(&mut self, count: usize) {
        adjust_gauge("rvoip_media_graph_sinks", self.sinks, count);
        self.sinks = count;
    }

    fn set_codec_group_count(&mut self, count: usize) {
        adjust_gauge("rvoip_media_graph_codec_groups", self.codec_groups, count);
        self.codec_groups = count;
    }
}

impl Drop for AggregateMetricsGuard {
    fn drop(&mut self) {
        adjust_gauge("rvoip_media_graph_sinks", self.sinks, 0);
        adjust_gauge("rvoip_media_graph_codec_groups", self.codec_groups, 0);
        metrics::gauge!("rvoip_media_graphs_active").decrement(1.0);
    }
}

fn adjust_gauge(name: &'static str, old: usize, new: usize) {
    if new > old {
        metrics::gauge!(name).increment((new - old) as f64);
    } else if old > new {
        metrics::gauge!(name).decrement((old - new) as f64);
    }
}

/// Ensures an aborted actor retains an accurate terminal snapshot.
struct SnapshotTerminalGuard {
    snapshot: RetainedSnapshot,
    sink_tasks: SinkTaskRegistry,
    completion: watch::Sender<Option<MediaGraphSourceState>>,
    route_statuses: RouteStatusRegistry,
}

impl Drop for SnapshotTerminalGuard {
    fn drop(&mut self) {
        let mut snapshot = (*read_snapshot(&self.snapshot)).clone();
        if snapshot.source_state == MediaGraphSourceState::Open {
            snapshot.source_state = MediaGraphSourceState::Aborted;
        }
        snapshot.sinks.clear();
        snapshot.codec_groups.clear();
        let terminal_state = snapshot.source_state;
        publish_snapshot(&self.snapshot, Arc::new(snapshot));

        let reason = match terminal_state {
            MediaGraphSourceState::Closed => MediaGraphRouteTerminalReason::SourceClosed,
            MediaGraphSourceState::Shutdown => MediaGraphRouteTerminalReason::GraphShutdown,
            MediaGraphSourceState::Open | MediaGraphSourceState::Aborted => {
                MediaGraphRouteTerminalReason::GraphAborted
            }
        };
        terminate_all_routes(&self.route_statuses, reason);

        let mut sink_tasks = self
            .sink_tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .drain(..)
            .collect::<Vec<_>>();
        for task in &sink_tasks {
            task.abort();
        }
        let completion = self.completion.clone();
        if sink_tasks.iter().all(AbortHandle::is_finished) {
            let _ = completion.send(Some(terminal_state));
        } else if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                while sink_tasks.iter().any(|task| !task.is_finished()) {
                    tokio::task::yield_now().await;
                }
                sink_tasks.clear();
                let _ = completion.send(Some(terminal_state));
            });
        } else {
            // Runtime teardown itself guarantees the sink tasks can no longer
            // execute; publish terminal state for any remaining synchronous
            // observers.
            let _ = completion.send(Some(terminal_state));
        }
    }
}

fn activate_route(statuses: &RouteStatusRegistry, route_id: &MediaRouteId) {
    if let Some(status) = statuses
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(route_id)
    {
        status.send_replace(MediaGraphRouteState::Active);
    }
}

fn terminate_route(
    statuses: &RouteStatusRegistry,
    route_id: &MediaRouteId,
    reason: MediaGraphRouteTerminalReason,
) {
    if let Some(status) = statuses
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(route_id)
    {
        status.send_replace(MediaGraphRouteState::Terminal(reason));
    }
}

fn terminate_all_routes(statuses: &RouteStatusRegistry, reason: MediaGraphRouteTerminalReason) {
    let statuses = statuses
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .drain()
        .map(|(_, status)| status)
        .collect::<Vec<_>>();
    for status in statuses {
        status.send_replace(MediaGraphRouteState::Terminal(reason));
    }
}

fn prune_sink_tasks(registry: &SinkTaskRegistry) {
    registry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .retain(|task| !task.is_finished());
}

/// Start a media graph task that owns `source` for its lifetime.
pub fn start_media_graph(
    source: mpsc::Receiver<MediaFrame>,
    source_codec: CodecInfo,
    policy: MediaGraphPolicy,
) -> Result<MediaGraphHandle> {
    start_media_graph_with_activity_interval(
        source,
        source_codec,
        policy,
        MEDIA_GRAPH_ACTIVITY_OBSERVATION_INTERVAL,
    )
}

fn start_media_graph_with_activity_interval(
    mut source: mpsc::Receiver<MediaFrame>,
    source_codec: CodecInfo,
    policy: MediaGraphPolicy,
    activity_observation_interval: Duration,
) -> Result<MediaGraphHandle> {
    let graph_id = MediaGraphId::new();
    validate_media_graph_codec(&source_codec)?;
    if activity_observation_interval.is_zero() {
        return Err(RvoipError::InvalidState(
            "media graph activity observation interval is invalid",
        ));
    }
    let initial_source_pt = payload_type_for_codec(&source_codec)
        .expect("validated media graph codec has an RTP payload type");
    let initial_snapshot = MediaGraphSnapshot {
        graph_id: graph_id.clone(),
        source_state: MediaGraphSourceState::Open,
        source_codec: source_codec.clone(),
        source_payload_type: initial_source_pt,
        source_frames: 0,
        sink_offers: 0,
        dropped_frames: 0,
        evictions: 0,
        transcode_operations: 0,
        transcode_errors: 0,
        sinks: Vec::new(),
        codec_groups: Vec::new(),
        recent_evictions: Vec::new(),
    };
    let latest_snapshot = Arc::new(StdRwLock::new(Arc::new(initial_snapshot)));
    let snapshot_for_task = Arc::clone(&latest_snapshot);
    let snapshot_in_flight = Arc::new(AtomicBool::new(false));
    let sink_tasks = Arc::new(Mutex::new(Vec::new()));
    let sink_tasks_for_actor = Arc::clone(&sink_tasks);
    let (completion_tx, completion_rx) = watch::channel(None);
    let (activity_tx, activity_rx) = watch::channel(None);
    let route_statuses: RouteStatusRegistry = Arc::new(Mutex::new(HashMap::new()));
    let route_statuses_for_actor = Arc::clone(&route_statuses);
    let sink_admission = Arc::new(SinkAdmissionState::new(policy.max_sinks));
    let graph_id_for_task = graph_id.clone();
    let (command_tx, mut command_rx) = mpsc::channel(CONTROL_QUEUE_CAPACITY);
    let (sink_event_tx, mut sink_event_rx) =
        mpsc::channel::<MediaRouteId>(SINK_EVENT_QUEUE_CAPACITY);

    // Construct the guard before spawning. If the task is aborted before its
    // first poll, dropping the unpolled future still terminalizes every route.
    let terminal_guard = SnapshotTerminalGuard {
        snapshot: Arc::clone(&snapshot_for_task),
        sink_tasks: Arc::clone(&sink_tasks_for_actor),
        completion: completion_tx,
        route_statuses: Arc::clone(&route_statuses_for_actor),
    };
    let task = tokio::spawn(async move {
        let _terminal_guard = terminal_guard;
        let mut aggregate_metrics = AggregateMetricsGuard::new();
        let mut source_codec = source_codec;
        let mut source_pt = initial_source_pt;
        let mut source_state = MediaGraphSourceState::Open;
        let mut sinks: HashMap<MediaRouteId, SinkRuntime> = HashMap::new();
        let mut groups: HashMap<CodecGroupKey, CodecGroup> = HashMap::new();
        let mut stats = GraphStats::default();
        let mut pre_sink_buffer = VecDeque::with_capacity(policy.pre_sink_buffer_frames);
        // Once a first sink has existed, later zero-sink periods deliberately
        // discard media rather than replaying stale RTP to a future attachment.
        let mut source_routing_started = false;
        let mut snapshot_dirty = false;
        let mut pending_activity_at = None;
        let mut snapshot_tick = tokio::time::interval(SNAPSHOT_PUBLISH_INTERVAL);
        snapshot_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut activity_tick = tokio::time::interval(activity_observation_interval);
        activity_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Consume interval's immediate first tick; the initial retained
        // snapshot was already published before spawning the actor.
        snapshot_tick.tick().await;
        activity_tick.tick().await;

        loop {
            tokio::select! {
                command = command_rx.recv() => {
                    let Some(command) = command else {
                        source_state = MediaGraphSourceState::Shutdown;
                        break;
                    };
                    match command {
                        Command::Add {
                            route_id,
                            codec,
                            target,
                            owner_liveness,
                            admission,
                        } => {
                            if owner_liveness.is_cancelled() {
                                terminate_route(
                                    &route_statuses_for_actor,
                                    &route_id,
                                    MediaGraphRouteTerminalReason::OwnerRemoved,
                                );
                                metrics::counter!(
                                    "rvoip_media_graph_owner_prunes_total",
                                    "phase" => "before-install"
                                )
                                .increment(1);
                                continue;
                            }
                            let Some(target_pt) = payload_type_for_codec(&codec) else {
                                terminate_route(
                                    &route_statuses_for_actor,
                                    &route_id,
                                    MediaGraphRouteTerminalReason::GraphAborted,
                                );
                                continue;
                            };
                            let first_sink = !source_routing_started;
                            let target_clock_rate_hz = codec.clock_rate_hz;
                            let status_route_id = route_id.clone();
                            let queue = Arc::new(SinkQueue::new(policy.sink_queue_frames));
                            let queue_for_task = Arc::clone(&queue);
                            let route_for_task = route_id.clone();
                            let event_tx = sink_event_tx.clone();
                            let task = tokio::spawn(async move {
                                while let Some(frame) = queue_for_task.receive().await {
                                    if target.send(frame).await.is_err() {
                                        let _ = event_tx.send(route_for_task.clone()).await;
                                        return;
                                    }
                                }
                            });
                            prune_sink_tasks(&sink_tasks_for_actor);
                            sink_tasks_for_actor
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .push(task.abort_handle());
                            let group_key = CodecGroupKey::new(&codec, target_pt);
                            groups.entry(group_key.clone())
                                .or_insert_with(|| CodecGroup::new(
                                    &source_codec,
                                    source_pt,
                                    codec.clone(),
                                    target_pt,
                                ))
                                .sinks.insert(route_id.clone());
                            sinks.insert(route_id, SinkRuntime {
                                target_codec: codec,
                                target_pt,
                                group_key,
                                owner_liveness,
                                _admission: admission,
                                clock: RtpClockTranslator::new(
                                    source_codec.clock_rate_hz,
                                    target_clock_rate_hz,
                                ),
                                queue,
                                task: task.abort_handle(),
                                history: VecDeque::new(),
                                rolling_drops: 0,
                                offered_frames: 0,
                                dropped_frames: 0,
                            });
                            source_routing_started = true;
                            aggregate_metrics.set_sink_count(sinks.len());
                            aggregate_metrics.set_codec_group_count(groups.len());
                            publish_actor_snapshot(
                                &snapshot_for_task,
                                &graph_id_for_task,
                                source_state,
                                &source_codec,
                                source_pt,
                                &sinks,
                                &groups,
                                &stats,
                            );
                            snapshot_dirty = false;
                            activate_route(&route_statuses_for_actor, &status_route_id);

                            if first_sink {
                                let mut terminal = Vec::new();
                                while let Some(frame) = pre_sink_buffer.pop_front() {
                                    terminal.extend(route_source_frame(
                                        frame,
                                        source_pt,
                                        &policy,
                                        &mut sinks,
                                        &mut groups,
                                        &mut stats,
                                    ));
                                }
                                snapshot_dirty = true;
                                if !terminal.is_empty() {
                                    aggregate_metrics.set_sink_count(sinks.len());
                                    aggregate_metrics.set_codec_group_count(groups.len());
                                    publish_actor_snapshot(
                                        &snapshot_for_task,
                                        &graph_id_for_task,
                                        source_state,
                                        &source_codec,
                                        source_pt,
                                        &sinks,
                                        &groups,
                                        &stats,
                                    );
                                    snapshot_dirty = false;
                                    for (route_id, reason) in terminal {
                                        terminate_route(
                                            &route_statuses_for_actor,
                                            &route_id,
                                            reason,
                                        );
                                    }
                                }
                            }
                        }
                        Command::Remove { route_id, ack } => {
                            let removed = remove_sink(&route_id, &mut sinks, &mut groups);
                            prune_sink_tasks(&sink_tasks_for_actor);
                            aggregate_metrics.set_sink_count(sinks.len());
                            aggregate_metrics.set_codec_group_count(groups.len());
                            if removed {
                                publish_actor_snapshot(
                                    &snapshot_for_task,
                                    &graph_id_for_task,
                                    source_state,
                                    &source_codec,
                                    source_pt,
                                    &sinks,
                                    &groups,
                                    &stats,
                                );
                                snapshot_dirty = false;
                                terminate_route(
                                    &route_statuses_for_actor,
                                    &route_id,
                                    MediaGraphRouteTerminalReason::OwnerRemoved,
                                );
                            }
                            if let Some(ack) = ack {
                                let _ = ack.send(removed);
                            }
                        }
                        Command::UpdateSourceCodec { codec, source_pt: new_source_pt, ack } => {
                            source_codec = codec;
                            source_pt = new_source_pt;
                            rebuild_transcoders(
                                &source_codec,
                                source_pt,
                                &mut sinks,
                                &mut groups,
                            );
                            publish_actor_snapshot(
                                &snapshot_for_task,
                                &graph_id_for_task,
                                source_state,
                                &source_codec,
                                source_pt,
                                &sinks,
                                &groups,
                                &stats,
                            );
                            snapshot_dirty = false;
                            let _ = ack.send(Ok(()));
                        }
                        Command::UpdateSinkCodec { route_id, codec, target_pt, ack } => {
                            let result = update_sink_group(
                                &route_id,
                                codec,
                                target_pt,
                                &source_codec,
                                source_pt,
                                &mut sinks,
                                &mut groups,
                            );
                            aggregate_metrics.set_codec_group_count(groups.len());
                            if result.is_ok() {
                                publish_actor_snapshot(
                                    &snapshot_for_task,
                                    &graph_id_for_task,
                                    source_state,
                                    &source_codec,
                                    source_pt,
                                    &sinks,
                                    &groups,
                                    &stats,
                                );
                                snapshot_dirty = false;
                            }
                            let _ = ack.send(result);
                        }
                        Command::UpdateRoute {
                            route_id,
                            source_codec: new_source_codec,
                            source_pt: new_source_pt,
                            target_codec,
                            target_pt,
                            ack,
                        } => {
                            source_codec = new_source_codec;
                            source_pt = new_source_pt;
                            rebuild_transcoders(
                                &source_codec,
                                source_pt,
                                &mut sinks,
                                &mut groups,
                            );
                            // Preserve the original API's success-on-missing-route behavior.
                            if sinks.contains_key(&route_id) {
                                let _ = update_sink_group(
                                    &route_id,
                                    target_codec,
                                    target_pt,
                                    &source_codec,
                                    source_pt,
                                    &mut sinks,
                                    &mut groups,
                                );
                            }
                            aggregate_metrics.set_codec_group_count(groups.len());
                            publish_actor_snapshot(
                                &snapshot_for_task,
                                &graph_id_for_task,
                                source_state,
                                &source_codec,
                                source_pt,
                                &sinks,
                                &groups,
                                &stats,
                            );
                            snapshot_dirty = false;
                            let _ = ack.send(Ok(()));
                        }
                        Command::Snapshot(reply) => {
                            // The waiting caller may have timed out while media
                            // was being processed. Skip stale diagnostic work.
                            if reply.is_closed() {
                                continue;
                            }
                            let snapshot = Arc::new(build_snapshot(
                                &graph_id_for_task,
                                source_state,
                                &source_codec,
                                source_pt,
                                &sinks,
                                &groups,
                                &stats,
                            ));
                            publish_snapshot(&snapshot_for_task, Arc::clone(&snapshot));
                            snapshot_dirty = false;
                            let _ = reply.send(snapshot);
                        }
                        Command::Shutdown => {
                            source_state = MediaGraphSourceState::Shutdown;
                            break;
                        }
                    }
                }
                closed_route = sink_event_rx.recv() => {
                    if let Some(route_id) = closed_route {
                        let removed = remove_sink(&route_id, &mut sinks, &mut groups);
                        aggregate_metrics.set_sink_count(sinks.len());
                        aggregate_metrics.set_codec_group_count(groups.len());
                        if removed {
                            publish_actor_snapshot(
                                &snapshot_for_task,
                                &graph_id_for_task,
                                source_state,
                                &source_codec,
                                source_pt,
                                &sinks,
                                &groups,
                                &stats,
                            );
                            snapshot_dirty = false;
                            terminate_route(
                                &route_statuses_for_actor,
                                &route_id,
                                MediaGraphRouteTerminalReason::TargetClosed,
                            );
                        }
                    }
                }
                frame = source.recv() => {
                    // A managed owner may have been dropped while the bounded
                    // control queue was full. Prune before routing the next
                    // frame so an orphan never receives further media.
                    let owner_removed =
                        prune_cancelled_owner_sinks(&mut sinks, &mut groups);
                    if !owner_removed.is_empty() {
                        aggregate_metrics.set_sink_count(sinks.len());
                        aggregate_metrics.set_codec_group_count(groups.len());
                        publish_actor_snapshot(
                            &snapshot_for_task,
                            &graph_id_for_task,
                            source_state,
                            &source_codec,
                            source_pt,
                            &sinks,
                            &groups,
                            &stats,
                        );
                        for route_id in owner_removed {
                            terminate_route(
                                &route_statuses_for_actor,
                                &route_id,
                                MediaGraphRouteTerminalReason::OwnerRemoved,
                            );
                        }
                    }
                    let Some(frame) = frame else {
                        if !pre_sink_buffer.is_empty() {
                            let dropped = pre_sink_buffer.len() as u64;
                            pre_sink_buffer.clear();
                            stats.dropped_frames =
                                stats.dropped_frames.saturating_add(dropped);
                            metrics::counter!(
                                "rvoip_media_graph_drops_total",
                                "reason" => "source-closed-before-sink"
                            )
                            .increment(dropped);
                        }
                        source_state = MediaGraphSourceState::Closed;
                        break;
                    };
                    stats.source_frames = stats.source_frames.saturating_add(1);
                    let observed_at = Utc::now();
                    pending_activity_at = Some(
                        pending_activity_at
                            .map_or(observed_at, |previous: DateTime<Utc>| previous.max(observed_at)),
                    );
                    snapshot_dirty = true;
                    metrics::counter!("rvoip_media_graph_source_frames_total").increment(1);
                    if !source_routing_started {
                        if policy.pre_sink_buffer_frames > 0 {
                            if pre_sink_buffer.len() == policy.pre_sink_buffer_frames {
                                pre_sink_buffer.pop_front();
                                stats.dropped_frames = stats.dropped_frames.saturating_add(1);
                                metrics::counter!(
                                    "rvoip_media_graph_drops_total",
                                    "reason" => "pre-sink-buffer-full"
                                )
                                .increment(1);
                            }
                            pre_sink_buffer.push_back(frame);
                        } else {
                            stats.dropped_frames = stats.dropped_frames.saturating_add(1);
                            metrics::counter!(
                                "rvoip_media_graph_drops_total",
                                "reason" => "pre-sink-buffer-disabled"
                            )
                            .increment(1);
                        }
                    } else {
                        let terminal = route_source_frame(
                            frame,
                            source_pt,
                            &policy,
                            &mut sinks,
                            &mut groups,
                            &mut stats,
                        );
                        aggregate_metrics.set_sink_count(sinks.len());
                        aggregate_metrics.set_codec_group_count(groups.len());
                        if !terminal.is_empty() {
                            publish_actor_snapshot(
                                &snapshot_for_task,
                                &graph_id_for_task,
                                source_state,
                                &source_codec,
                                source_pt,
                                &sinks,
                                &groups,
                                &stats,
                            );
                            snapshot_dirty = false;
                            for (route_id, reason) in terminal {
                                terminate_route(
                                    &route_statuses_for_actor,
                                    &route_id,
                                    reason,
                                );
                            }
                        }
                    }
                }
                _ = snapshot_tick.tick() => {
                    // Periodic maintenance guarantees convergence even when
                    // the source is idle and Drop could not enqueue Remove.
                    let owner_removed =
                        prune_cancelled_owner_sinks(&mut sinks, &mut groups);
                    prune_sink_tasks(&sink_tasks_for_actor);
                    if !owner_removed.is_empty() {
                        aggregate_metrics.set_sink_count(sinks.len());
                        aggregate_metrics.set_codec_group_count(groups.len());
                    }
                    if snapshot_dirty || !owner_removed.is_empty() {
                        publish_snapshot(
                            &snapshot_for_task,
                            Arc::new(build_snapshot(
                                &graph_id_for_task,
                                source_state,
                                &source_codec,
                                source_pt,
                                &sinks,
                                &groups,
                                &stats,
                            )),
                        );
                        snapshot_dirty = false;
                    }
                    for route_id in owner_removed {
                        terminate_route(
                            &route_statuses_for_actor,
                            &route_id,
                            MediaGraphRouteTerminalReason::OwnerRemoved,
                        );
                    }
                }
                _ = activity_tick.tick() => {
                    publish_pending_activity(
                        &activity_tx,
                        &mut pending_activity_at,
                        stats.source_frames,
                    );
                }
            }
        }

        // Preserve the last accepted frame even when the source closes or a
        // graceful graph shutdown happens before the next coalescing tick.
        // Lifecycle ownership is revalidated by the Orchestrator before this
        // retained observation can become an authoritative event.
        publish_pending_activity(&activity_tx, &mut pending_activity_at, stats.source_frames);
        sinks.clear();
        groups.clear();
        aggregate_metrics.set_sink_count(0);
        aggregate_metrics.set_codec_group_count(0);
        publish_snapshot(
            &snapshot_for_task,
            Arc::new(build_snapshot(
                &graph_id_for_task,
                source_state,
                &source_codec,
                source_pt,
                &sinks,
                &groups,
                &stats,
            )),
        );
        debug!(graph_id = %graph_id_for_task, ?source_state, "rvoip media graph stopped");
    });

    Ok(MediaGraphHandle {
        graph_id,
        commands: command_tx,
        abort: task.abort_handle(),
        latest_snapshot,
        snapshot_in_flight,
        completion: completion_rx,
        activity: activity_rx,
        route_statuses,
        sink_admission,
    })
}

fn publish_pending_activity(
    sender: &watch::Sender<Option<MediaGraphActivityObservation>>,
    pending_at: &mut Option<DateTime<Utc>>,
    source_frames: u64,
) {
    let Some(observed_at) = pending_at.take() else {
        return;
    };
    sender.send_replace(Some(MediaGraphActivityObservation {
        source_frames,
        observed_at,
    }));
}

fn publish_actor_snapshot(
    retained: &RetainedSnapshot,
    graph_id: &MediaGraphId,
    source_state: MediaGraphSourceState,
    source_codec: &CodecInfo,
    source_pt: u8,
    sinks: &HashMap<MediaRouteId, SinkRuntime>,
    groups: &HashMap<CodecGroupKey, CodecGroup>,
    stats: &GraphStats,
) {
    publish_snapshot(
        retained,
        Arc::new(build_snapshot(
            graph_id,
            source_state,
            source_codec,
            source_pt,
            sinks,
            groups,
            stats,
        )),
    );
}

/// Route one already-accounted source frame. Payload transcoding remains once
/// per codec group, while each sink advances its own RTP clock so a group
/// re-key cannot reset that route's timestamp epoch.
fn route_source_frame(
    frame: MediaFrame,
    source_pt: u8,
    policy: &MediaGraphPolicy,
    sinks: &mut HashMap<MediaRouteId, SinkRuntime>,
    groups: &mut HashMap<CodecGroupKey, CodecGroup>,
    stats: &mut GraphStats,
) -> Vec<(MediaRouteId, MediaGraphRouteTerminalReason)> {
    let now = Instant::now();
    let mut evict = Vec::new();
    let mut closed = Vec::new();
    let is_telephone_event = frame.payload_type == Some(DEFAULT_TELEPHONE_EVENT_PT);

    for group in groups.values_mut() {
        group.source_frames_routed = group.source_frames_routed.saturating_add(1);
        let mut grouped = frame.clone();
        if !is_telephone_event {
            if let Some(transcoder) = group.transcoder.as_mut() {
                group.transcode_operations = group.transcode_operations.saturating_add(1);
                stats.transcode_operations = stats.transcode_operations.saturating_add(1);
                metrics::counter!(
                    "rvoip_media_graph_transcodes_total",
                    "target_payload_type" => group.target_pt.to_string()
                )
                .increment(1);
                match transcoder.transcode(&frame.payload) {
                    Ok(payload) => {
                        grouped.payload = payload.into();
                        grouped.payload_type = Some(group.target_pt);
                    }
                    Err(error) => {
                        stats.transcode_errors = stats.transcode_errors.saturating_add(1);
                        warn!(
                            %error,
                            source_pt,
                            target_pt = group.target_pt,
                            "media graph transcode failed"
                        );
                        metrics::counter!("rvoip_media_graph_transcode_errors_total").increment(1);
                        continue;
                    }
                }
            }
        }

        for route_id in &group.sinks {
            let Some(sink) = sinks.get_mut(route_id) else {
                continue;
            };
            let mut routed = grouped.clone();
            if !is_telephone_event {
                routed.timestamp_rtp = sink.clock.translate(frame.timestamp_rtp);
            }
            let offer = sink.queue.offer(routed);
            if offer == OfferResult::Closed {
                closed.push(route_id.clone());
                continue;
            }
            let dropped = offer == OfferResult::DroppedOldest;
            stats.sink_offers = stats.sink_offers.saturating_add(1);
            metrics::counter!("rvoip_media_graph_frames_total").increment(1);
            if dropped {
                stats.dropped_frames = stats.dropped_frames.saturating_add(1);
                metrics::counter!(
                    "rvoip_media_graph_drops_total",
                    "reason" => "queue-full"
                )
                .increment(1);
            }
            if sink.record_offer(now, dropped, policy) {
                evict.push(route_id.clone());
            }
        }
    }

    closed.sort();
    closed.dedup();
    evict.sort();
    evict.dedup();
    let mut terminal = Vec::with_capacity(closed.len() + evict.len());
    for route_id in closed {
        if remove_sink(&route_id, sinks, groups) {
            terminal.push((route_id, MediaGraphRouteTerminalReason::TargetClosed));
        }
    }
    for route_id in evict {
        if let Some(sink) = sinks.get(&route_id) {
            stats.record_eviction(route_id.clone(), sink);
            metrics::counter!(
                "rvoip_media_graph_evictions_total",
                "reason" => "slow-consumer"
            )
            .increment(1);
        }
        if remove_sink(&route_id, sinks, groups) {
            terminal.push((route_id, MediaGraphRouteTerminalReason::SlowConsumerEvicted));
        }
    }
    terminal
}

fn update_sink_group(
    route_id: &MediaRouteId,
    codec: CodecInfo,
    target_pt: u8,
    source_codec: &CodecInfo,
    source_pt: u8,
    sinks: &mut HashMap<MediaRouteId, SinkRuntime>,
    groups: &mut HashMap<CodecGroupKey, CodecGroup>,
) -> Result<()> {
    let Some(sink) = sinks.get_mut(route_id) else {
        return Err(RvoipError::InvalidState("media graph sink not found"));
    };
    if let Some(group) = groups.get_mut(&sink.group_key) {
        group.sinks.remove(route_id);
    }
    let group_key = CodecGroupKey::new(&codec, target_pt);
    sink.clock
        .reconfigure(source_codec.clock_rate_hz, codec.clock_rate_hz);
    sink.target_codec = codec.clone();
    sink.target_pt = target_pt;
    sink.group_key = group_key.clone();
    groups.retain(|_, group| !group.sinks.is_empty());
    groups
        .entry(group_key)
        .or_insert_with(|| CodecGroup::new(source_codec, source_pt, codec, target_pt))
        .sinks
        .insert(route_id.clone());
    Ok(())
}

fn rebuild_transcoders(
    source_codec: &CodecInfo,
    source_pt: u8,
    sinks: &mut HashMap<MediaRouteId, SinkRuntime>,
    groups: &mut HashMap<CodecGroupKey, CodecGroup>,
) {
    for sink in sinks.values_mut() {
        sink.clock
            .reconfigure(source_codec.clock_rate_hz, sink.target_codec.clock_rate_hz);
    }
    for group in groups.values_mut() {
        group.transcoder = make_transcoder(
            source_codec,
            source_pt,
            &group.target_codec,
            group.target_pt,
        );
    }
}

fn remove_sink(
    route_id: &MediaRouteId,
    sinks: &mut HashMap<MediaRouteId, SinkRuntime>,
    groups: &mut HashMap<CodecGroupKey, CodecGroup>,
) -> bool {
    let Some(sink) = sinks.remove(route_id) else {
        return false;
    };
    if let Some(group) = groups.get_mut(&sink.group_key) {
        group.sinks.remove(route_id);
    }
    groups.retain(|_, group| !group.sinks.is_empty());
    true
}

/// Remove managed routes whose sole owner has cancelled them. The scan is
/// bounded by `MediaGraphPolicy::max_sinks`; it never creates a fallback task
/// or channel when the control queue is saturated.
fn prune_cancelled_owner_sinks(
    sinks: &mut HashMap<MediaRouteId, SinkRuntime>,
    groups: &mut HashMap<CodecGroupKey, CodecGroup>,
) -> Vec<MediaRouteId> {
    let mut cancelled = sinks
        .iter()
        .filter_map(|(route_id, sink)| sink.owner_liveness.is_cancelled().then(|| route_id.clone()))
        .collect::<Vec<_>>();
    cancelled.sort();
    cancelled.retain(|route_id| remove_sink(route_id, sinks, groups));
    if !cancelled.is_empty() {
        metrics::counter!(
            "rvoip_media_graph_owner_prunes_total",
            "phase" => "installed"
        )
        .increment(cancelled.len() as u64);
    }
    cancelled
}

fn codec_for_payload_type(payload_type: u8) -> Option<CodecInfo> {
    let (name, clock_rate_hz) = match payload_type {
        0 => ("pcmu", 8_000),
        8 => ("pcma", 8_000),
        18 => ("g729", 8_000),
        111 => ("opus", 48_000),
        _ => return None,
    };
    Some(CodecInfo {
        name: name.into(),
        clock_rate_hz,
        channels: 1,
        fmtp: None,
    })
}

fn build_snapshot(
    graph_id: &MediaGraphId,
    source_state: MediaGraphSourceState,
    source_codec: &CodecInfo,
    source_pt: u8,
    sinks: &HashMap<MediaRouteId, SinkRuntime>,
    groups: &HashMap<CodecGroupKey, CodecGroup>,
    stats: &GraphStats,
) -> MediaGraphSnapshot {
    let mut sink_snapshots: Vec<_> = sinks
        .iter()
        .map(|(route_id, sink)| {
            let (rolling_samples, rolling_drops) = sink.rolling_drop_counts();
            MediaGraphSinkSnapshot {
                route_id: route_id.clone(),
                target_codec: sink.target_codec.clone(),
                target_payload_type: sink.target_pt,
                queue_depth: sink.queue.depth(),
                queue_capacity: sink.queue.capacity,
                offered_frames: sink.offered_frames,
                dropped_frames: sink.dropped_frames,
                rolling_samples,
                rolling_drops,
                rolling_drop_ratio: if rolling_samples == 0 {
                    0.0
                } else {
                    rolling_drops as f32 / rolling_samples as f32
                },
            }
        })
        .collect();
    sink_snapshots.sort_by(|a, b| a.route_id.cmp(&b.route_id));

    let mut codec_groups: Vec<_> = groups
        .values()
        .map(|group| {
            let mut sink_routes: Vec<_> = group.sinks.iter().cloned().collect();
            sink_routes.sort();
            MediaGraphCodecGroupSnapshot {
                target_codec: group.target_codec.clone(),
                target_payload_type: group.target_pt,
                sink_routes,
                transcoding: group.transcoder.is_some(),
                source_frames_routed: group.source_frames_routed,
                transcode_operations: group.transcode_operations,
            }
        })
        .collect();
    codec_groups.sort_by(|a, b| {
        (
            a.target_payload_type,
            a.target_codec.name.as_str(),
            a.target_codec.clock_rate_hz,
            a.target_codec.channels,
            a.target_codec.fmtp.as_deref(),
        )
            .cmp(&(
                b.target_payload_type,
                b.target_codec.name.as_str(),
                b.target_codec.clock_rate_hz,
                b.target_codec.channels,
                b.target_codec.fmtp.as_deref(),
            ))
    });

    MediaGraphSnapshot {
        graph_id: graph_id.clone(),
        source_state,
        source_codec: source_codec.clone(),
        source_payload_type: source_pt,
        source_frames: stats.source_frames,
        sink_offers: stats.sink_offers,
        dropped_frames: stats.dropped_frames,
        evictions: stats.evictions,
        transcode_operations: stats.transcode_operations,
        transcode_errors: stats.transcode_errors,
        sinks: sink_snapshots,
        codec_groups,
        recent_evictions: stats.recent_evictions.iter().cloned().collect(),
    }
}

fn publish_snapshot(shared: &RetainedSnapshot, snapshot: Arc<MediaGraphSnapshot>) {
    *shared
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = snapshot;
}

fn read_snapshot(snapshot: &RetainedSnapshot) -> Arc<MediaGraphSnapshot> {
    Arc::clone(
        &snapshot
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;
    use chrono::Utc;

    use super::*;
    use crate::ids::StreamId;
    use crate::stream::StreamKind;

    fn codec(name: &str, clock_rate: u32) -> CodecInfo {
        CodecInfo {
            name: name.into(),
            clock_rate_hz: clock_rate,
            channels: 1,
            fmtp: None,
        }
    }

    fn frame(value: u8) -> MediaFrame {
        frame_at(value, value as u32 * 160)
    }

    fn frame_at(value: u8, timestamp_rtp: u32) -> MediaFrame {
        frame_at_pt(value, timestamp_rtp, 0)
    }

    fn frame_at_pt(value: u8, timestamp_rtp: u32, payload_type: u8) -> MediaFrame {
        MediaFrame {
            stream_id: StreamId::from_string("strm_media_graph_test"),
            kind: StreamKind::Audio,
            payload: Bytes::from(vec![value; 160]),
            timestamp_rtp,
            captured_at: Utc::now(),
            payload_type: Some(payload_type),
        }
    }

    async fn wait_until(mut predicate: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !predicate() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("condition did not become true");
    }

    async fn wait_for_route_state(status: &MediaGraphRouteStatus, expected: MediaGraphRouteState) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while status.state() != expected {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("route state did not converge");
    }

    #[tokio::test]
    async fn one_source_reaches_multiple_sinks() {
        let (source_tx, source_rx) = mpsc::channel(4);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let graph_id = graph.id().clone();
        let (a_tx, mut a_rx) = mpsc::channel(4);
        let (b_tx, mut b_rx) = mpsc::channel(4);
        graph.add_sink(codec("pcmu", 8_000), a_tx).unwrap();
        graph.add_sink(codec("pcmu", 8_000), b_tx).unwrap();
        let snapshot = graph.snapshot().await;
        assert_eq!(snapshot.graph_id, graph_id);
        assert_eq!(snapshot.sinks.len(), 2);
        assert_eq!(snapshot.codec_groups.len(), 1);

        source_tx.send(frame(7)).await.unwrap();
        assert_eq!(a_rx.recv().await.unwrap().payload[0], 7);
        assert_eq!(b_rx.recv().await.unwrap().payload[0], 7);
        graph.shutdown();
    }

    #[tokio::test]
    async fn source_activity_is_retained_and_coalesced_per_interval() {
        let (source_tx, source_rx) = mpsc::channel(64);
        for value in 0..32 {
            source_tx.try_send(frame(value)).unwrap();
        }
        let graph = start_media_graph_with_activity_interval(
            source_rx,
            codec("pcmu", 8_000),
            MediaGraphPolicy::default(),
            Duration::from_millis(40),
        )
        .unwrap();
        let mut activity = graph.subscribe_activity();
        tokio::time::timeout(Duration::from_secs(1), activity.changed())
            .await
            .expect("coalesced observation deadline")
            .expect("activity publisher remains live");
        let first = activity
            .borrow_and_update()
            .clone()
            .expect("activity observation");
        assert_eq!(first.source_frames, 32);
        assert!(
            tokio::time::timeout(Duration::from_millis(60), activity.changed())
                .await
                .is_err(),
            "idle ticks do not manufacture activity"
        );

        source_tx.send(frame(33)).await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), activity.changed())
            .await
            .expect("next observation deadline")
            .expect("activity publisher remains live");
        let second = activity
            .borrow_and_update()
            .clone()
            .expect("next activity observation");
        assert_eq!(second.source_frames, 33);
        assert!(second.observed_at >= first.observed_at);
        graph.shutdown_and_wait().await.unwrap();
    }

    #[test]
    fn zero_activity_observation_interval_is_rejected() {
        let (_source_tx, source_rx) = mpsc::channel(1);
        assert!(matches!(
            start_media_graph_with_activity_interval(
                source_rx,
                codec("pcmu", 8_000),
                MediaGraphPolicy::default(),
                Duration::ZERO,
            ),
            Err(RvoipError::InvalidState(
                "media graph activity observation interval is invalid"
            ))
        ));
    }

    #[tokio::test]
    async fn buffered_first_frame_waits_for_initial_sink_registration() {
        let (source_tx, source_rx) = mpsc::channel(1);
        source_tx.send(frame(42)).await.unwrap();
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        // Give the actor opportunities to race the already-ready source. The
        // initial-route gate must keep the frame buffered.
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        let (target_tx, mut target_rx) = mpsc::channel(1);
        graph.add_sink(codec("pcmu", 8_000), target_tx).unwrap();

        let received = tokio::time::timeout(Duration::from_secs(1), target_rx.recv())
            .await
            .expect("initial media was not routed")
            .expect("initial sink closed");
        assert_eq!(received.payload[0], 42);
        graph.shutdown_and_wait().await.unwrap();
    }

    #[tokio::test]
    async fn pre_sink_buffer_is_bounded_drop_oldest_and_flushes_in_order() {
        let policy = MediaGraphPolicy {
            sink_queue_frames: 4,
            pre_sink_buffer_frames: 3,
            ..Default::default()
        };
        let (source_tx, source_rx) = mpsc::channel(8);
        for value in 0..6 {
            source_tx.send(frame(value)).await.unwrap();
        }
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), policy).unwrap();

        // Establish that all six ready source frames reached the bounded
        // pre-sink buffer before registering the first sink.
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if graph.snapshot().await.source_frames == 6 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("source did not drain into the pre-sink buffer");

        let (target_tx, mut target_rx) = mpsc::channel(8);
        let route = graph
            .add_managed_sink(codec("pcmu", 8_000), target_tx)
            .unwrap();
        wait_for_route_state(&route.status(), MediaGraphRouteState::Active).await;
        for expected in 3..6 {
            assert_eq!(target_rx.recv().await.unwrap().payload[0], expected);
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(20), target_rx.recv())
                .await
                .is_err()
        );
        assert_eq!(graph.snapshot().await.dropped_frames, 3);
        graph.shutdown_and_wait().await.unwrap();
    }

    #[tokio::test]
    async fn closed_source_with_buffered_media_converges_and_accounts_for_drops() {
        let (source_tx, source_rx) = mpsc::channel(4);
        for value in 0..3 {
            source_tx.send(frame(value)).await.unwrap();
        }
        drop(source_tx);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();

        assert_eq!(
            graph.wait_closed().await.unwrap(),
            MediaGraphSourceState::Closed
        );
        let snapshot = graph.latest_snapshot();
        assert_eq!(snapshot.source_frames, 3);
        assert_eq!(snapshot.dropped_frames, 3);
        assert_eq!(snapshot.source_state, MediaGraphSourceState::Closed);
    }

    #[tokio::test]
    async fn removing_one_sink_does_not_stop_others() {
        let (source_tx, source_rx) = mpsc::channel(4);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let (a_tx, mut a_rx) = mpsc::channel(4);
        let (b_tx, mut b_rx) = mpsc::channel(4);
        let a = graph.add_sink(codec("pcmu", 8_000), a_tx).unwrap();
        graph.add_sink(codec("pcmu", 8_000), b_tx).unwrap();
        graph.snapshot().await;
        graph.remove_sink(a);
        assert_eq!(graph.snapshot().await.sinks.len(), 1);

        source_tx.send(frame(9)).await.unwrap();
        let removed = tokio::time::timeout(Duration::from_millis(50), a_rx.recv()).await;
        assert!(matches!(removed, Ok(None) | Err(_)));
        assert_eq!(b_rx.recv().await.unwrap().payload[0], 9);
        graph.shutdown();
    }

    #[tokio::test]
    async fn acknowledged_removal_reports_route_existence() {
        let (_source_tx, source_rx) = mpsc::channel(1);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let (target_tx, mut target_rx) = mpsc::channel(1);
        let route = graph.add_sink(codec("pcmu", 8_000), target_tx).unwrap();
        graph.snapshot().await;

        assert!(graph.remove_sink_and_wait(route.clone()).await.unwrap());
        assert!(graph.latest_snapshot().sinks.is_empty());
        assert!(!graph.remove_sink_and_wait(route).await.unwrap());
        assert!(target_rx.recv().await.is_none());
        graph.shutdown_and_wait().await.unwrap();
    }

    #[tokio::test]
    async fn managed_owner_drop_removes_route_without_status_clone_ownership() {
        let (_source_tx, source_rx) = mpsc::channel(1);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let (target_tx, _target_rx) = mpsc::channel(1);
        let route = graph
            .add_managed_sink(codec("pcmu", 8_000), target_tx)
            .unwrap();
        let status = route.status();
        let observer = status.clone();
        wait_for_route_state(&status, MediaGraphRouteState::Active).await;
        assert_eq!(graph.latest_snapshot().sinks.len(), 1);

        drop(route);
        assert_eq!(
            status.wait_terminal().await,
            MediaGraphRouteTerminalReason::OwnerRemoved
        );
        assert_eq!(
            observer.state(),
            MediaGraphRouteState::Terminal(MediaGraphRouteTerminalReason::OwnerRemoved)
        );
        assert!(graph.latest_snapshot().sinks.is_empty());
        assert!(graph
            .route_statuses
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty());
        graph.shutdown_and_wait().await.unwrap();
    }

    #[tokio::test]
    async fn managed_owner_drop_converges_when_control_queue_is_saturated() {
        let (source_tx, source_rx) = mpsc::channel(1);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let (target_tx, mut target_rx) = mpsc::channel(1);
        let route = graph
            .add_managed_sink(codec("pcmu", 8_000), target_tx)
            .unwrap();
        let status = route.status();
        wait_for_route_state(&status, MediaGraphRouteState::Active).await;

        // This test runs on Tokio's current-thread runtime. With no await in
        // this loop, the actor cannot drain commands while every bounded slot
        // is filled with a live snapshot request.
        let mut snapshot_receivers = Vec::with_capacity(CONTROL_QUEUE_CAPACITY);
        for _ in 0..CONTROL_QUEUE_CAPACITY {
            let (reply, receive) = oneshot::channel();
            assert!(graph.commands.try_send(Command::Snapshot(reply)).is_ok());
            snapshot_receivers.push(receive);
        }
        let (overflow, _receive) = oneshot::channel();
        assert!(matches!(
            graph.commands.try_send(Command::Snapshot(overflow)),
            Err(mpsc::error::TrySendError::Full(_))
        ));
        assert_eq!(graph.commands.capacity(), 0);

        // Drop's Remove fast path is necessarily rejected, but cancellation
        // remains visible to the actor. Frame activity must prune the route
        // before routing this next packet.
        drop(route);
        source_tx.try_send(frame(77)).unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), status.wait_terminal())
                .await
                .expect("cancelled route did not converge"),
            MediaGraphRouteTerminalReason::OwnerRemoved
        );

        let snapshot = graph.snapshot().await;
        assert!(snapshot.sinks.is_empty());
        assert!(snapshot.codec_groups.is_empty());
        assert_eq!(snapshot.sink_offers, 0);
        assert_eq!(graph.sink_admission.in_use.load(Ordering::Acquire), 0);
        assert!(graph
            .route_statuses
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty());
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), target_rx.recv()).await,
            Ok(None)
        ));
        drop(snapshot_receivers);
        graph.shutdown_and_wait().await.unwrap();
    }

    #[test]
    fn default_sink_limit_reserves_direct_fanout_headroom() {
        assert_eq!(
            MediaGraphPolicy::default().max_sinks,
            DEFAULT_MEDIA_GRAPH_MAX_SINKS
        );
        assert_eq!(DEFAULT_MEDIA_GRAPH_MAX_SINKS, 1_024);
    }

    #[tokio::test]
    async fn sink_admission_accepts_the_boundary_rejects_one_more_and_recovers() {
        let policy = MediaGraphPolicy {
            max_sinks: 2,
            ..Default::default()
        };
        let (_source_tx, source_rx) = mpsc::channel(1);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), policy).unwrap();
        let (first_tx, _first_rx) = mpsc::channel(1);
        let (second_tx, _second_rx) = mpsc::channel(1);
        let first = graph.add_sink(codec("pcmu", 8_000), first_tx).unwrap();
        graph.add_sink(codec("pcmu", 8_000), second_tx).unwrap();

        let (rejected_tx, _rejected_rx) = mpsc::channel(1);
        assert!(matches!(
            graph.add_sink(codec("pcmu", 8_000), rejected_tx),
            Err(RvoipError::AdmissionRejected(
                "media graph maximum sink count reached"
            ))
        ));
        let at_limit = graph.snapshot().await;
        assert_eq!(at_limit.sinks.len(), 2);
        assert_eq!(at_limit.codec_groups.len(), 1);
        assert_eq!(graph.sink_admission.in_use.load(Ordering::Acquire), 2);
        assert_eq!(
            graph
                .route_statuses
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            2
        );

        assert!(graph.remove_sink_and_wait(first).await.unwrap());
        assert_eq!(graph.sink_admission.in_use.load(Ordering::Acquire), 1);
        let (replacement_tx, _replacement_rx) = mpsc::channel(1);
        graph
            .add_sink(codec("pcmu", 8_000), replacement_tx)
            .unwrap();
        let recovered = graph.snapshot().await;
        assert_eq!(recovered.sinks.len(), 2);
        assert_eq!(recovered.codec_groups.len(), 1);
        assert_eq!(graph.sink_admission.in_use.load(Ordering::Acquire), 2);
        graph.shutdown_and_wait().await.unwrap();
    }

    #[tokio::test]
    async fn repeated_managed_add_remove_does_not_retain_status_registry_entries() {
        let (_source_tx, source_rx) = mpsc::channel(1);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();

        for iteration in 0..32 {
            let (target_tx, _target_rx) = mpsc::channel(1);
            let route = graph
                .add_managed_sink(codec("pcmu", 8_000), target_tx)
                .unwrap();
            let status = route.status();
            wait_for_route_state(&status, MediaGraphRouteState::Active).await;
            if iteration % 2 == 0 {
                drop(route);
            } else {
                assert!(route.remove().await.unwrap());
            }
            assert_eq!(
                status.wait_terminal().await,
                MediaGraphRouteTerminalReason::OwnerRemoved
            );
            assert!(graph
                .route_statuses
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty());
        }
        assert!(graph.latest_snapshot().sinks.is_empty());
        graph.shutdown_and_wait().await.unwrap();
    }

    #[tokio::test]
    async fn target_close_reports_terminal_reason_and_late_status_clone() {
        let (source_tx, source_rx) = mpsc::channel(2);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let (target_tx, target_rx) = mpsc::channel(1);
        let route = graph
            .add_managed_sink(codec("pcmu", 8_000), target_tx)
            .unwrap();
        let status = route.status();
        wait_for_route_state(&status, MediaGraphRouteState::Active).await;
        drop(target_rx);
        source_tx.send(frame(1)).await.unwrap();

        assert_eq!(
            status.wait_terminal().await,
            MediaGraphRouteTerminalReason::TargetClosed
        );
        let late = status.clone();
        assert_eq!(
            late.state(),
            MediaGraphRouteState::Terminal(MediaGraphRouteTerminalReason::TargetClosed)
        );
        assert!(graph.latest_snapshot().sinks.is_empty());
        graph.shutdown_and_wait().await.unwrap();
    }

    #[tokio::test]
    async fn full_queue_drops_oldest_frame() {
        let queue = SinkQueue::new(2);
        assert_eq!(queue.offer(frame(1)), OfferResult::Enqueued);
        assert_eq!(queue.offer(frame(2)), OfferResult::Enqueued);
        assert_eq!(queue.offer(frame(3)), OfferResult::DroppedOldest);
        assert_eq!(queue.depth(), 2);
        assert_eq!(queue.receive().await.unwrap().payload[0], 2);
        assert_eq!(queue.depth(), 1);
        assert_eq!(queue.receive().await.unwrap().payload[0], 3);
        queue.close();
        assert!(queue.receive().await.is_none());
    }

    #[tokio::test]
    async fn eviction_is_strictly_greater_than_twenty_five_percent_over_ten_seconds() {
        let policy = MediaGraphPolicy {
            max_sinks: DEFAULT_MEDIA_GRAPH_MAX_SINKS,
            sink_queue_frames: 1,
            pre_sink_buffer_frames: 10,
            eviction_window: Duration::from_secs(10),
            eviction_drop_ratio: 0.25,
            minimum_eviction_samples: 4,
        };
        let (target, _receiver) = mpsc::channel::<MediaFrame>(1);
        let task = tokio::spawn(async move { drop(target) });
        let target_codec = codec("pcmu", 8_000);
        let mut sink = SinkRuntime {
            group_key: CodecGroupKey::new(&target_codec, 0),
            target_codec,
            target_pt: 0,
            owner_liveness: Arc::new(RouteOwnerLiveness::default()),
            _admission: Arc::new(SinkAdmissionState::new(1)).try_acquire().unwrap(),
            clock: RtpClockTranslator::new(8_000, 8_000),
            queue: Arc::new(SinkQueue::new(1)),
            task: task.abort_handle(),
            history: VecDeque::new(),
            rolling_drops: 0,
            offered_frames: 0,
            dropped_frames: 0,
        };
        let start = Instant::now();
        assert!(!sink.record_offer(start, true, &policy));
        assert!(!sink.record_offer(start + Duration::from_secs(1), false, &policy));
        assert!(!sink.record_offer(start + Duration::from_secs(2), false, &policy));
        // Exactly 25% is retained, not evicted.
        assert!(!sink.record_offer(start + Duration::from_secs(3), false, &policy));
        assert_eq!(sink.rolling_drop_counts(), (4, 1));
        // The sample at exactly the ten-second boundary remains in the window.
        assert!(!sink.record_offer(start + Duration::from_secs(10), false, &policy));
        // One nanosecond later the original drop is pruned, so the new drop is
        // only one of the five current samples.
        assert!(!sink.record_offer(
            start + Duration::from_secs(10) + Duration::from_nanos(1),
            true,
            &policy,
        ));
        assert_eq!(sink.rolling_drop_counts(), (5, 1));
        // Two of six current samples is 33%, which is strictly over policy.
        assert!(sink.record_offer(
            start + Duration::from_secs(10) + Duration::from_nanos(2),
            true,
            &policy,
        ));
        assert_eq!(sink.rolling_drop_counts(), (6, 2));
    }

    #[tokio::test]
    async fn slow_sink_is_evicted_and_reported() {
        let policy = MediaGraphPolicy {
            max_sinks: DEFAULT_MEDIA_GRAPH_MAX_SINKS,
            sink_queue_frames: 1,
            pre_sink_buffer_frames: 10,
            eviction_window: Duration::from_secs(10),
            eviction_drop_ratio: 0.25,
            minimum_eviction_samples: 4,
        };
        let (source_tx, source_rx) = mpsc::channel(64);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), policy).unwrap();
        let (target_tx, _target_rx) = mpsc::channel(1);
        let route = graph.add_sink(codec("pcmu", 8_000), target_tx).unwrap();
        graph.snapshot().await;
        for value in 0..32 {
            source_tx.send(frame(value)).await.unwrap();
        }
        wait_until(|| graph.latest_snapshot().evictions == 1).await;
        let snapshot = graph.snapshot().await;
        assert!(snapshot.sinks.is_empty());
        assert!(snapshot.dropped_frames > 0);
        assert_eq!(snapshot.recent_evictions[0].route_id, route);
        assert_eq!(
            snapshot.recent_evictions[0].reason,
            MediaGraphEvictionReason::SlowConsumer
        );
        graph.shutdown();
    }

    #[tokio::test]
    async fn managed_slow_sink_reports_eviction_terminal_reason() {
        let policy = MediaGraphPolicy {
            max_sinks: DEFAULT_MEDIA_GRAPH_MAX_SINKS,
            sink_queue_frames: 1,
            pre_sink_buffer_frames: 10,
            eviction_window: Duration::from_secs(10),
            eviction_drop_ratio: 0.25,
            minimum_eviction_samples: 4,
        };
        let (source_tx, source_rx) = mpsc::channel(64);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), policy).unwrap();
        let (target_tx, _target_rx) = mpsc::channel(1);
        let route = graph
            .add_managed_sink(codec("pcmu", 8_000), target_tx)
            .unwrap();
        let status = route.status();
        wait_for_route_state(&status, MediaGraphRouteState::Active).await;
        for value in 0..32 {
            source_tx.send(frame(value)).await.unwrap();
        }

        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), status.wait_terminal())
                .await
                .expect("slow sink was not evicted"),
            MediaGraphRouteTerminalReason::SlowConsumerEvicted
        );
        assert!(graph.latest_snapshot().sinks.is_empty());
        graph.shutdown_and_wait().await.unwrap();
    }

    #[tokio::test]
    async fn transcodes_once_per_codec_group_and_preserves_timestamps() {
        let (source_tx, source_rx) = mpsc::channel(4);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let (a_tx, mut a_rx) = mpsc::channel(4);
        let (b_tx, mut b_rx) = mpsc::channel(4);
        graph.add_sink(codec("pcma", 8_000), a_tx).unwrap();
        graph.add_sink(codec("pcma", 8_000), b_tx).unwrap();
        graph.snapshot().await;

        source_tx.send(frame_at(0x7f, 42_424)).await.unwrap();
        let a = a_rx.recv().await.unwrap();
        let b = b_rx.recv().await.unwrap();
        assert_eq!(a.timestamp_rtp, 42_424);
        assert_eq!(b.timestamp_rtp, 42_424);
        assert_eq!(a.stream_id, b.stream_id);
        assert_eq!(a.payload, b.payload);
        let snapshot = graph.snapshot().await;
        assert_eq!(snapshot.transcode_operations, 1);
        assert_eq!(snapshot.codec_groups[0].transcode_operations, 1);
        assert_eq!(snapshot.codec_groups[0].sink_routes.len(), 2);
        graph.shutdown();
    }

    #[tokio::test]
    async fn frame_order_and_rtp_timestamp_continuity_are_preserved() {
        let (source_tx, source_rx) = mpsc::channel(8);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let (target_tx, mut target_rx) = mpsc::channel(8);
        graph.add_sink(codec("pcmu", 8_000), target_tx).unwrap();
        graph.snapshot().await;

        let timestamps = [u32::MAX - 159, 0, 160, 320];
        for (value, timestamp) in timestamps.into_iter().enumerate() {
            source_tx
                .send(frame_at(value as u8, timestamp))
                .await
                .unwrap();
        }
        for (value, timestamp) in timestamps.into_iter().enumerate() {
            let received = target_rx.recv().await.unwrap();
            assert_eq!(received.payload[0], value as u8);
            assert_eq!(received.timestamp_rtp, timestamp);
        }
        graph.shutdown();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_add_remove_leaves_no_routes_or_groups() {
        let (_source_tx, source_rx) = mpsc::channel(4);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let mut tasks = Vec::new();
        for _ in 0..64 {
            let graph = graph.clone();
            tasks.push(tokio::spawn(async move {
                let (target, _receiver) = mpsc::channel(1);
                let route = graph.add_sink(codec("pcmu", 8_000), target).unwrap();
                assert!(graph.remove_sink(route));
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        let snapshot = graph.snapshot().await;
        assert!(snapshot.sinks.is_empty());
        assert!(snapshot.codec_groups.is_empty());
        graph.shutdown();
    }

    #[tokio::test]
    async fn source_close_closes_sinks_and_retains_final_snapshot() {
        let (source_tx, source_rx) = mpsc::channel(1);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let (target_tx, mut target_rx) = mpsc::channel(1);
        graph.add_sink(codec("pcmu", 8_000), target_tx).unwrap();
        graph.snapshot().await;
        drop(source_tx);

        wait_until(|| graph.abort_handle().is_finished()).await;
        assert!(target_rx.recv().await.is_none());
        let snapshot = graph.snapshot().await;
        assert_eq!(snapshot.source_state, MediaGraphSourceState::Closed);
        assert!(snapshot.sinks.is_empty());
        assert!(snapshot.codec_groups.is_empty());
    }

    #[tokio::test]
    async fn empty_source_close_is_observed_before_first_sink() {
        let (source_tx, source_rx) = mpsc::channel(1);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        drop(source_tx);
        assert_eq!(
            graph.wait_closed().await.unwrap(),
            MediaGraphSourceState::Closed
        );
        assert!(graph.abort_handle().is_finished());
    }

    #[tokio::test]
    async fn shutdown_and_abort_cleanup_all_sink_tasks() {
        let (_source_tx, source_rx) = mpsc::channel(1);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let (target_tx, mut target_rx) = mpsc::channel(1);
        graph.add_sink(codec("pcmu", 8_000), target_tx).unwrap();
        graph.snapshot().await;
        assert_eq!(
            graph.shutdown_and_wait().await.unwrap(),
            MediaGraphSourceState::Shutdown
        );
        assert!(graph.abort_handle().is_finished());
        assert!(target_rx.recv().await.is_none());
        assert_eq!(
            graph.snapshot().await.source_state,
            MediaGraphSourceState::Shutdown
        );

        let (_source_tx, source_rx) = mpsc::channel(1);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let (target_tx, mut target_rx) = mpsc::channel(1);
        graph.add_sink(codec("pcmu", 8_000), target_tx).unwrap();
        graph.snapshot().await;
        graph.abort_handle().abort();
        assert_eq!(
            graph.wait_closed().await.unwrap(),
            MediaGraphSourceState::Aborted
        );
        assert!(graph.abort_handle().is_finished());
        assert!(target_rx.recv().await.is_none());
        let snapshot = graph.snapshot().await;
        assert_eq!(snapshot.source_state, MediaGraphSourceState::Aborted);
        assert!(snapshot.sinks.is_empty());
        assert!(snapshot.codec_groups.is_empty());
    }

    #[tokio::test]
    async fn managed_routes_report_shutdown_source_close_and_abort() {
        // Graceful graph shutdown.
        let (_source_tx, source_rx) = mpsc::channel(1);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let (target_tx, _target_rx) = mpsc::channel(1);
        let route = graph
            .add_managed_sink(codec("pcmu", 8_000), target_tx)
            .unwrap();
        let status = route.status();
        wait_for_route_state(&status, MediaGraphRouteState::Active).await;
        assert_eq!(
            graph.shutdown_and_wait().await.unwrap(),
            MediaGraphSourceState::Shutdown
        );
        assert_eq!(
            status.wait_terminal().await,
            MediaGraphRouteTerminalReason::GraphShutdown
        );

        // Natural source closure.
        let (source_tx, source_rx) = mpsc::channel(1);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let (target_tx, _target_rx) = mpsc::channel(1);
        let route = graph
            .add_managed_sink(codec("pcmu", 8_000), target_tx)
            .unwrap();
        let status = route.status();
        wait_for_route_state(&status, MediaGraphRouteState::Active).await;
        drop(source_tx);
        assert_eq!(
            status.wait_terminal().await,
            MediaGraphRouteTerminalReason::SourceClosed
        );
        assert_eq!(
            graph.wait_closed().await.unwrap(),
            MediaGraphSourceState::Closed
        );

        // Forced actor abort, including a status clone subscribing after the
        // terminal update has already been retained.
        let (_source_tx, source_rx) = mpsc::channel(1);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let (target_tx, _target_rx) = mpsc::channel(1);
        let route = graph
            .add_managed_sink(codec("pcmu", 8_000), target_tx)
            .unwrap();
        let status = route.status();
        wait_for_route_state(&status, MediaGraphRouteState::Active).await;
        graph.abort_handle().abort();
        assert_eq!(
            status.wait_terminal().await,
            MediaGraphRouteTerminalReason::GraphAborted
        );
        let late = status.clone();
        assert_eq!(
            late.state(),
            MediaGraphRouteState::Terminal(MediaGraphRouteTerminalReason::GraphAborted)
        );
        assert_eq!(
            graph.wait_closed().await.unwrap(),
            MediaGraphSourceState::Aborted
        );
    }

    #[tokio::test]
    async fn cloned_handles_observe_the_same_terminal_convergence() {
        let (_source_tx, source_rx) = mpsc::channel(1);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let waiter_a = graph.clone();
        let waiter_b = graph.clone();
        let a = tokio::spawn(async move { waiter_a.wait_closed().await.unwrap() });
        let b = tokio::spawn(async move { waiter_b.wait_closed().await.unwrap() });

        assert_eq!(
            graph.shutdown_and_wait().await.unwrap(),
            MediaGraphSourceState::Shutdown
        );
        assert_eq!(a.await.unwrap(), MediaGraphSourceState::Shutdown);
        assert_eq!(b.await.unwrap(), MediaGraphSourceState::Shutdown);
    }

    #[tokio::test]
    async fn source_and_sink_codec_updates_are_independent() {
        let (_source_tx, source_rx) = mpsc::channel(1);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let (target_tx, _target_rx) = mpsc::channel(1);
        let route = graph.add_sink(codec("pcmu", 8_000), target_tx).unwrap();
        graph.snapshot().await;

        graph
            .update_source_codec(codec("pcma", 8_000))
            .await
            .unwrap();
        // Update acknowledgement is also a retained-snapshot barrier.
        let snapshot = graph.latest_snapshot();
        assert_eq!(snapshot.source_payload_type, 8);
        assert_eq!(snapshot.sinks[0].target_payload_type, 0);
        assert!(snapshot.codec_groups[0].transcoding);

        graph
            .update_sink_codec(route.clone(), codec("opus", 48_000))
            .await
            .unwrap();
        let snapshot = graph.latest_snapshot();
        assert_eq!(snapshot.source_payload_type, 8);
        assert_eq!(snapshot.sinks[0].target_payload_type, 111);
        assert_eq!(snapshot.codec_groups[0].target_payload_type, 111);

        graph.update_route(route, 0, 8).await.unwrap();
        let snapshot = graph.latest_snapshot();
        assert_eq!(snapshot.source_payload_type, 0);
        assert_eq!(snapshot.sinks[0].target_payload_type, 8);
        graph.shutdown();
    }

    #[test]
    fn rtp_clock_translation_is_wrap_safe_in_both_directions() {
        let mut upsample = RtpClockTranslator::new(8_000, 48_000);
        let first = u32::MAX - 159;
        let upsampled = [
            upsample.translate(first),
            upsample.translate(0),
            upsample.translate(160),
        ];
        assert_eq!(upsampled[0], first);
        assert_eq!(upsampled[1].wrapping_sub(upsampled[0]), 960);
        assert_eq!(upsampled[2].wrapping_sub(upsampled[1]), 960);

        let mut downsample = RtpClockTranslator::new(48_000, 8_000);
        let first = u32::MAX - 959;
        let downsampled = [
            downsample.translate(first),
            downsample.translate(0),
            downsample.translate(960),
        ];
        assert_eq!(downsampled[0], first);
        assert_eq!(downsampled[1].wrapping_sub(downsampled[0]), 160);
        assert_eq!(downsampled[2].wrapping_sub(downsampled[1]), 160);
    }

    #[tokio::test]
    async fn each_codec_group_uses_its_own_rtp_clock() {
        let (source_tx, source_rx) = mpsc::channel(4);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let (pcmu_tx, mut pcmu_rx) = mpsc::channel(4);
        let (opus_tx, mut opus_rx) = mpsc::channel(4);
        graph.add_sink(codec("pcmu", 8_000), pcmu_tx).unwrap();
        graph.add_sink(codec("opus", 48_000), opus_tx).unwrap();
        graph.snapshot().await;

        source_tx.send(frame_at(0xff, 10_000)).await.unwrap();
        source_tx.send(frame_at(0xff, 10_160)).await.unwrap();
        let pcmu_first = pcmu_rx.recv().await.unwrap();
        let pcmu_second = pcmu_rx.recv().await.unwrap();
        let opus_first = opus_rx.recv().await.unwrap();
        let opus_second = opus_rx.recv().await.unwrap();
        assert_eq!(
            pcmu_second
                .timestamp_rtp
                .wrapping_sub(pcmu_first.timestamp_rtp),
            160
        );
        assert_eq!(
            opus_second
                .timestamp_rtp
                .wrapping_sub(opus_first.timestamp_rtp),
            960
        );
        graph.shutdown();
    }

    #[tokio::test]
    async fn sink_rekey_preserves_timestamp_epoch_across_8k_and_48k_targets() {
        let (source_tx, source_rx) = mpsc::channel(8);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let (target_tx, mut target_rx) = mpsc::channel(8);
        let route = graph.add_sink(codec("opus", 48_000), target_tx).unwrap();
        graph.snapshot().await;

        source_tx.send(frame_at(0x7f, 10_000)).await.unwrap();
        source_tx.send(frame_at(0x7f, 10_160)).await.unwrap();
        let first = target_rx.recv().await.unwrap();
        let second = target_rx.recv().await.unwrap();
        assert_eq!(second.timestamp_rtp.wrapping_sub(first.timestamp_rtp), 960);

        let mut rekeyed_opus = codec("opus", 48_000);
        rekeyed_opus.fmtp = Some("minptime=20;useinbandfec=1".into());
        graph
            .update_sink_codec(route.clone(), rekeyed_opus)
            .await
            .unwrap();
        source_tx.send(frame_at(0x7f, 10_320)).await.unwrap();
        let after_fmtp_rekey = target_rx.recv().await.unwrap();
        assert_eq!(
            after_fmtp_rekey
                .timestamp_rtp
                .wrapping_sub(second.timestamp_rtp),
            960
        );

        graph
            .update_sink_codec(route, codec("pcmu", 8_000))
            .await
            .unwrap();
        source_tx.send(frame_at(0x7f, 10_480)).await.unwrap();
        let after_48k_to_8k = target_rx.recv().await.unwrap();
        assert_eq!(
            after_48k_to_8k
                .timestamp_rtp
                .wrapping_sub(after_fmtp_rekey.timestamp_rtp),
            160
        );
        graph.shutdown_and_wait().await.unwrap();
    }

    #[tokio::test]
    async fn source_reconfiguration_preserves_target_clock_across_8k_48k_8k() {
        let (source_tx, source_rx) = mpsc::channel(8);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let (target_tx, mut target_rx) = mpsc::channel(8);
        graph.add_sink(codec("opus", 48_000), target_tx).unwrap();
        graph.snapshot().await;

        source_tx.send(frame_at(0x7f, 10_000)).await.unwrap();
        source_tx.send(frame_at(0x7f, 10_160)).await.unwrap();
        let first = target_rx.recv().await.unwrap();
        let second = target_rx.recv().await.unwrap();
        assert_eq!(second.timestamp_rtp.wrapping_sub(first.timestamp_rtp), 960);

        graph
            .update_source_codec(codec("opus", 48_000))
            .await
            .unwrap();
        source_tx
            .send(frame_at_pt(0x11, 11_120, 111))
            .await
            .unwrap();
        let after_8k_to_48k = target_rx.recv().await.unwrap();
        assert_eq!(
            after_8k_to_48k
                .timestamp_rtp
                .wrapping_sub(second.timestamp_rtp),
            960
        );

        graph
            .update_source_codec(codec("pcmu", 8_000))
            .await
            .unwrap();
        source_tx.send(frame_at(0x7f, 11_280)).await.unwrap();
        let after_48k_to_8k = target_rx.recv().await.unwrap();
        assert_eq!(
            after_48k_to_8k
                .timestamp_rtp
                .wrapping_sub(after_8k_to_48k.timestamp_rtp),
            960
        );
        graph.shutdown_and_wait().await.unwrap();
    }

    #[tokio::test]
    async fn normalized_codec_identity_controls_grouping() {
        let (_source_tx, source_rx) = mpsc::channel(1);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let mut opus_a = codec("OPUS", 48_000);
        opus_a.fmtp = Some("minptime=10; useinbandfec=1".into());
        let mut opus_a_reordered = codec("opus", 48_000);
        opus_a_reordered.fmtp = Some("useinbandfec=1;minptime=10".into());
        let mut opus_b = codec("opus", 48_000);
        opus_b.fmtp = Some("minptime=20;useinbandfec=1".into());
        let (a_tx, _a_rx) = mpsc::channel(1);
        let (b_tx, _b_rx) = mpsc::channel(1);
        let (c_tx, _c_rx) = mpsc::channel(1);
        graph.add_sink(opus_a, a_tx).unwrap();
        graph.add_sink(opus_a_reordered, b_tx).unwrap();
        graph.add_sink(opus_b, c_tx).unwrap();

        let mut group_sizes: Vec<_> = graph
            .snapshot()
            .await
            .codec_groups
            .into_iter()
            .map(|group| group.sink_routes.len())
            .collect();
        group_sizes.sort_unstable();
        assert_eq!(group_sizes, vec![1, 2]);
        graph.shutdown();
    }

    #[test]
    fn configured_transcoder_honors_canonical_opus_mono() {
        let source = codec("pcmu", 8_000);
        let mut target = codec("opus", 48_000);
        target.fmtp = Some("maxaveragebitrate=32000;cbr=1".into());
        let session = ConfiguredTranscodingSession::new(&source, 0, &target, 111).unwrap();
        let source_info = session.source_codec.get_info();
        let target_info = session.target_codec.get_info();
        assert_eq!(source_info.sample_rate, 8_000);
        assert_eq!(source_info.channels, 1);
        assert_eq!(target_info.sample_rate, 48_000);
        assert_eq!(target_info.channels, 1);
    }

    #[tokio::test]
    async fn compatibility_update_rejects_unknown_payload_types_atomically() {
        let (_source_tx, source_rx) = mpsc::channel(1);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let (target_tx, _target_rx) = mpsc::channel(1);
        let route = graph.add_sink(codec("pcma", 8_000), target_tx).unwrap();
        let before = graph.snapshot().await;

        assert!(matches!(
            graph.update_route(route.clone(), 127, 8).await,
            Err(RvoipError::UnsupportedCodec(_))
        ));
        assert!(matches!(
            graph.update_route(route, 0, 127).await,
            Err(RvoipError::UnsupportedCodec(_))
        ));
        let after = graph.snapshot().await;
        assert_eq!(after.source_codec, before.source_codec);
        assert_eq!(after.source_payload_type, before.source_payload_type);
        assert_eq!(after.sinks[0].target_codec, before.sinks[0].target_codec);
        assert_eq!(
            after.sinks[0].target_payload_type,
            before.sinks[0].target_payload_type
        );
        graph.shutdown();
    }

    #[tokio::test]
    async fn frame_hot_path_does_not_rebuild_retained_snapshot() {
        let (source_tx, source_rx) = mpsc::channel(2);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), Default::default()).unwrap();
        let (target_tx, mut target_rx) = mpsc::channel(2);
        graph.add_sink(codec("pcmu", 8_000), target_tx).unwrap();
        let baseline = graph.snapshot_arc().await;

        source_tx.send(frame(1)).await.unwrap();
        target_rx.recv().await.unwrap();
        let retained = graph.latest_snapshot_arc();
        assert!(Arc::ptr_eq(&baseline, &retained));

        let explicit = graph.snapshot_arc().await;
        assert!(!Arc::ptr_eq(&baseline, &explicit));
        assert_eq!(explicit.source_frames, 1);
        graph.shutdown();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn snapshot_flood_is_coalesced_without_starving_media() {
        let policy = MediaGraphPolicy {
            sink_queue_frames: 64,
            ..Default::default()
        };
        let (source_tx, source_rx) = mpsc::channel(64);
        let graph = start_media_graph(source_rx, codec("pcmu", 8_000), policy).unwrap();
        let (target_tx, mut target_rx) = mpsc::channel(64);
        graph.add_sink(codec("pcmu", 8_000), target_tx).unwrap();
        graph.snapshot().await;

        let mut readers = Vec::new();
        for _ in 0..32 {
            let graph = graph.clone();
            readers.push(tokio::spawn(async move {
                for _ in 0..100 {
                    let _ = graph.snapshot_arc().await;
                }
            }));
        }
        for value in 0..20 {
            source_tx.send(frame(value)).await.unwrap();
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            for _ in 0..20 {
                target_rx.recv().await.expect("media sink closed");
            }
        })
        .await
        .expect("snapshot traffic starved media");
        for reader in readers {
            reader.await.unwrap();
        }
        assert_eq!(graph.snapshot().await.source_frames, 20);
        graph.shutdown();
    }
}
