//! Media executor selection and the inline receive seam.
//!
//! The media executor is chosen once per process with
//! [`install_media_executor_config`]. [`MediaExecutorMode::V1`], the default,
//! keeps the per-call task graph: the socket task publishes every packet on
//! the transport broadcast, the session task accounts it and publishes it
//! again, and the media layer decodes it on its own task.
//!
//! [`MediaExecutorMode::V2`] processes an authenticated inbound RTP packet run
//! to completion on the task that owns its socket: receive, SRTP unprotect in
//! the receive buffer, stream accounting, then a hand-off to a per-session
//! [`InlineMediaSink`] (the media layer's decoder) without crossing two
//! broadcast queues and two more task wakeups. A session that cannot take the
//! inline path (a codec the sink cannot decode, a sink that arrives after
//! media already flowed, a bridge that needs the packet broadcast) falls back
//! to v1 for the rest of its life and is counted in
//! [`InlineExecutorCounters::fallbacks`].
//!
//! Under v2 the following parts can be switched off one by one (each is on in
//! [`MediaExecutorConfig::v2`], and every one is off under v1):
//!
//! - [`MediaExecutorConfig::lane_poller`]: the receive loops of a media lane
//!   run inside one poller task per lane worker, so one task wakeup serves
//!   every socket that became ready together.
//! - [`MediaExecutorConfig::lane_timer`]: futures hosted on a lane poller
//!   wait on the poller's lane clock (one runtime timer per poller) instead of
//!   one runtime timer each. Needs the lane poller.
//! - [`MediaExecutorConfig::lane_tx`]: [`spawn_on_media_lane`] hosts paced
//!   TX tasks on their lane's poller. Needs the lane poller.
//! - [`MediaExecutorConfig::tx_batch`]: callers of the synchronous TX path
//!   (`RtpSendHandle::try_send_now`) send frames that fall due together in
//!   one `sendmmsg` on Linux.
//! - [`MediaExecutorConfig::fast_ssrc_hash`]: SSRC-keyed maps on the packet
//!   path (SRTP rollover and replay state, session streams) use a keyed
//!   SplitMix64 hasher instead of SipHash.
//!
//! Every decision is read when an object is built (a transport starts, a
//! session or SRTP context is created, a poller starts), never per packet.

use std::cell::Cell;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

use bytes::Bytes;

use crate::packet::{RtpHeader, RtpPacket};

/// Which media executor the process runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MediaExecutorMode {
    /// The per-call task graph (default).
    #[default]
    V1,
    /// Inline run-to-completion receive on the socket owner, lane pollers
    /// and the synchronous TX path, as selected by the
    /// [`MediaExecutorConfig`] switches.
    V2,
}

impl MediaExecutorMode {
    /// A stable lower-case name (`"v1"`, `"v2"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::V1 => "v1",
            Self::V2 => "v2",
        }
    }
}

impl std::fmt::Display for MediaExecutorMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for MediaExecutorMode {
    type Err = MediaExecutorConfigError;

    /// `v1` or `v2`, ignoring ASCII case and surrounding whitespace.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        if value.eq_ignore_ascii_case("v1") {
            Ok(Self::V1)
        } else if value.eq_ignore_ascii_case("v2") {
            Ok(Self::V2)
        } else {
            Err(MediaExecutorConfigError::UnknownMode {
                value: value.to_owned(),
            })
        }
    }
}

/// Process-wide media executor configuration.
///
/// [`Self::v1`] (also [`Default`]) is the per-call task graph every earlier
/// release ran. [`Self::v2`] is the inline executor with every switch on. A
/// switch only takes effect under [`MediaExecutorMode::V2`]; under v1 every
/// switch reads as off whatever its field says.
///
/// ```
/// use rvoip_rtp_core::executor::{MediaExecutorConfig, MediaExecutorMode};
///
/// let v2 = MediaExecutorConfig::v2();
/// assert_eq!(v2.mode, MediaExecutorMode::V2);
/// assert!(v2.lane_poller && v2.tx_batch);
///
/// // v2 with receive loops on their own tasks (which also turns off the lane
/// // clock and lane-hosted TX, which need the poller).
/// let config = MediaExecutorConfig::v2().with_lane_poller(false);
/// config.validate().unwrap();
/// assert!(!config.lane_timer_active());
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct MediaExecutorConfig {
    /// The executor.
    pub mode: MediaExecutorMode,
    /// Host v2 receive loops on one poller task per media lane worker.
    pub lane_poller: bool,
    /// Lane-hosted futures wait on the poller's lane clock. Needs
    /// [`Self::lane_poller`].
    pub lane_timer: bool,
    /// [`spawn_on_media_lane`] hosts its future on the lane poller. Needs
    /// [`Self::lane_poller`].
    pub lane_tx: bool,
    /// Frames of one call that fall due together leave in one `sendmmsg`
    /// (Linux) on the synchronous TX path.
    pub tx_batch: bool,
    /// SSRC-keyed maps on the packet path use the keyed fast hasher.
    pub fast_ssrc_hash: bool,
}

impl Default for MediaExecutorConfig {
    fn default() -> Self {
        Self::v1()
    }
}

impl MediaExecutorConfig {
    /// The per-call task graph: every switch off.
    pub fn v1() -> Self {
        Self {
            mode: MediaExecutorMode::V1,
            lane_poller: false,
            lane_timer: false,
            lane_tx: false,
            tx_batch: false,
            fast_ssrc_hash: false,
        }
    }

    /// The inline executor with every switch on.
    pub fn v2() -> Self {
        Self {
            mode: MediaExecutorMode::V2,
            lane_poller: true,
            lane_timer: true,
            lane_tx: true,
            tx_batch: true,
            fast_ssrc_hash: true,
        }
    }

    /// Set [`Self::mode`].
    pub fn with_mode(mut self, mode: MediaExecutorMode) -> Self {
        self.mode = mode;
        self
    }

    /// Set [`Self::lane_poller`].
    pub fn with_lane_poller(mut self, enabled: bool) -> Self {
        self.lane_poller = enabled;
        self
    }

    /// Set [`Self::lane_timer`].
    pub fn with_lane_timer(mut self, enabled: bool) -> Self {
        self.lane_timer = enabled;
        self
    }

    /// Set [`Self::lane_tx`].
    pub fn with_lane_tx(mut self, enabled: bool) -> Self {
        self.lane_tx = enabled;
        self
    }

    /// Set [`Self::tx_batch`].
    pub fn with_tx_batch(mut self, enabled: bool) -> Self {
        self.tx_batch = enabled;
        self
    }

    /// Set [`Self::fast_ssrc_hash`].
    pub fn with_fast_ssrc_hash(mut self, enabled: bool) -> Self {
        self.fast_ssrc_hash = enabled;
        self
    }

    /// Check the configuration without installing it.
    ///
    /// A switch whose prerequisite is off is not an error: it reads as off
    /// (switching the lane poller off is the documented way to keep receive
    /// loops on their own tasks). Rules for switches that need a specific
    /// media runtime layout are checked here.
    pub fn validate(&self) -> Result<(), MediaExecutorConfigError> {
        Ok(())
    }

    fn is_v2(&self) -> bool {
        self.mode == MediaExecutorMode::V2
    }

    /// Whether v2 receive loops run on the lane poller.
    pub fn lane_poller_active(&self) -> bool {
        self.is_v2() && self.lane_poller
    }

    /// Whether lane-hosted futures use the lane clock.
    pub fn lane_timer_active(&self) -> bool {
        self.lane_poller_active() && self.lane_timer
    }

    /// Whether [`spawn_on_media_lane`] hosts futures on the lane poller.
    pub fn lane_tx_active(&self) -> bool {
        self.lane_poller_active() && self.lane_tx
    }

    /// Whether the synchronous TX path batches co-due frames.
    pub fn tx_batch_active(&self) -> bool {
        self.is_v2() && self.tx_batch
    }

    /// Whether SSRC-keyed maps built now use the fast hasher.
    pub fn fast_ssrc_hash_active(&self) -> bool {
        self.is_v2() && self.fast_ssrc_hash
    }
}

/// Why a [`MediaExecutorConfig`] was rejected.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MediaExecutorConfigError {
    /// A mode name other than `v1` or `v2`.
    #[error("media executor mode must be v1 or v2, got {value:?}")]
    UnknownMode {
        /// The rejected name.
        value: String,
    },
    /// A different configuration is already installed or already in use.
    ///
    /// The media executor is process-wide and every decision is captured
    /// when an object is built, so it is configured once, before the first
    /// media transport or session.
    #[error("the media executor is already configured as {active:?}")]
    AlreadyConfigured {
        /// The configuration in effect.
        active: Box<MediaExecutorConfig>,
    },
}

static PROCESS_CONFIG: OnceLock<MediaExecutorConfig> = OnceLock::new();

thread_local! {
    static THREAD_CONFIG_OVERRIDE: std::cell::RefCell<Option<MediaExecutorConfig>> =
        const { std::cell::RefCell::new(None) };
    static THREAD_HAS_OVERRIDE: Cell<bool> = const { Cell::new(false) };
}

/// Install the media executor configuration for this process.
///
/// Call this once at startup, before the first media transport or session;
/// the first read otherwise fixes the default ([`MediaExecutorConfig::v1`]).
/// Installing a configuration equal to the one in effect succeeds and changes
/// nothing, so several components may install the same configuration. A
/// different one returns [`MediaExecutorConfigError::AlreadyConfigured`].
pub fn install_media_executor_config(
    config: MediaExecutorConfig,
) -> Result<(), MediaExecutorConfigError> {
    install_into(&PROCESS_CONFIG, config)
}

fn install_into(
    slot: &OnceLock<MediaExecutorConfig>,
    config: MediaExecutorConfig,
) -> Result<(), MediaExecutorConfigError> {
    config.validate()?;
    let active = slot.get_or_init(|| config.clone());
    if *active == config {
        Ok(())
    } else {
        Err(MediaExecutorConfigError::AlreadyConfigured {
            active: Box::new(active.clone()),
        })
    }
}

/// The media executor configuration in effect for objects built on this
/// thread. Reading it fixes the process default if none was installed.
pub fn media_executor_config() -> MediaExecutorConfig {
    if THREAD_HAS_OVERRIDE.with(Cell::get) {
        if let Some(config) = THREAD_CONFIG_OVERRIDE.with(|slot| slot.borrow().clone()) {
            return config;
        }
    }
    process_config().clone()
}

fn process_config() -> &'static MediaExecutorConfig {
    PROCESS_CONFIG.get_or_init(MediaExecutorConfig::default)
}

/// Evaluate `read` against the configuration in effect on this thread,
/// without cloning the process configuration.
#[inline]
fn with_config<R>(read: impl Fn(&MediaExecutorConfig) -> R) -> R {
    if THREAD_HAS_OVERRIDE.with(Cell::get) {
        if let Some(value) = THREAD_CONFIG_OVERRIDE.with(|slot| slot.borrow().as_ref().map(&read)) {
            return value;
        }
    }
    read(process_config())
}

/// The executor in effect on this thread.
pub fn media_executor_mode() -> MediaExecutorMode {
    with_config(|config| config.mode)
}

/// Whether v2 receive loops run on the per-lane poller. Read when a
/// transport starts.
pub fn lane_poller_enabled() -> bool {
    with_config(MediaExecutorConfig::lane_poller_active)
}

/// Whether lane-hosted futures use the poller's lane clock. Read when a
/// poller starts and when a media sleep is created.
pub fn lane_timer_enabled() -> bool {
    with_config(MediaExecutorConfig::lane_timer_active)
}

/// Whether [`spawn_on_media_lane`] hosts futures on the lane poller.
pub fn lane_tx_enabled() -> bool {
    with_config(MediaExecutorConfig::lane_tx_active)
}

/// Whether callers of the synchronous TX path should batch co-due frames.
pub fn tx_batch_enabled() -> bool {
    with_config(MediaExecutorConfig::tx_batch_active)
}

/// Whether SSRC-keyed maps built now use the fast hasher.
pub fn fast_ssrc_hash_enabled() -> bool {
    with_config(MediaExecutorConfig::fast_ssrc_hash_active)
}

/// Restores the previous per-thread override when dropped.
#[doc(hidden)]
pub struct MediaExecutorConfigOverride {
    previous: Option<MediaExecutorConfig>,
}

impl Drop for MediaExecutorConfigOverride {
    fn drop(&mut self) {
        let previous = self.previous.take();
        THREAD_HAS_OVERRIDE.with(|flag| flag.set(previous.is_some()));
        THREAD_CONFIG_OVERRIDE.with(|slot| *slot.borrow_mut() = previous);
    }
}

/// Test seam: use `config` for objects built on this thread only, until the
/// guard drops. A `#[tokio::test]` (current-thread runtime) builds its
/// sessions and pollers on the test thread, so parallel tests can run v1 and
/// v2 side by side.
#[doc(hidden)]
pub fn override_media_executor_config_on_this_thread(
    config: MediaExecutorConfig,
) -> MediaExecutorConfigOverride {
    let previous = THREAD_CONFIG_OVERRIDE.with(|slot| slot.borrow_mut().replace(config));
    THREAD_HAS_OVERRIDE.with(|flag| flag.set(true));
    MediaExecutorConfigOverride { previous }
}

/// Run `future` on `runtime`, hosted on one of the runtime's lane pollers
/// when [`MediaExecutorConfig::lane_tx`] is active (one task wakeup then
/// serves every hosted future and socket that became ready together, and its
/// timers use the lane clock), otherwise as its own task.
///
/// The future is detached either way, like a dropped `JoinHandle`. A hosted
/// future that panics ends alone (the poller logs it); one that is still
/// pending when its runtime shuts down is dropped with the poller, as a
/// task's future is dropped with its runtime.
pub fn spawn_on_media_lane<F>(runtime: &tokio::runtime::Handle, future: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    if lane_tx_enabled() {
        match crate::lane_poller::host_task_on_lane(runtime, future) {
            Ok(_detached) => record_lane_hosted_task(),
            Err(future) => drop(runtime.spawn(future)),
        }
    } else {
        drop(runtime.spawn(future));
    }
}

/// One authenticated, source-accepted RTP media packet, borrowed from the
/// socket owner's receive buffer for the duration of one inline call.
///
/// The fields are exactly those the v1 path carries in
/// `RtpEvent::MediaReceived`, so [`Self::to_packet`] rebuilds the same
/// minimal `RtpPacket` the v1 session task publishes.
#[derive(Clone, Copy, Debug)]
pub struct InlineRtpPacket<'a> {
    pub payload_type: u8,
    pub sequence_number: u16,
    pub timestamp: u32,
    pub marker: bool,
    pub ssrc: u32,
    pub padding_size: u8,
    pub payload: &'a [u8],
    pub source: SocketAddr,
}

impl InlineRtpPacket<'_> {
    /// The minimal header the v1 session task reconstructs for every packet:
    /// no CSRCs, no extension.
    pub fn minimal_header(&self) -> RtpHeader {
        RtpHeader {
            version: 2,
            padding: self.padding_size != 0,
            extension: false,
            cc: 0,
            marker: self.marker,
            payload_type: self.payload_type,
            sequence_number: self.sequence_number,
            timestamp: self.timestamp,
            ssrc: self.ssrc,
            csrc: Vec::new(),
            extensions: None,
        }
    }

    /// Copy into an owned packet. One allocation; used only when a packet
    /// must outlive the receive buffer (reorder hold, fallback broadcast).
    pub fn to_packet(&self) -> RtpPacket {
        RtpPacket {
            header: self.minimal_header(),
            payload: Bytes::copy_from_slice(self.payload),
            padding_size: self.padding_size,
        }
    }

    /// Bytes the v1 session counts for this packet: a 12-byte minimal header
    /// plus payload and padding.
    pub fn accounted_size(&self) -> usize {
        12 + self.payload.len() + usize::from(self.padding_size)
    }
}

/// What the inline sink wants after a packet or deadline.
pub enum InlineSinkVerdict {
    /// Keep the sink on the inline path.
    Continue,
    /// Leave the inline path for good. `held` are packets the sink accepted
    /// but had not played out, oldest first; the session republishes them as
    /// `PacketReceived` (followed by the current packet, when demotion
    /// happened on a packet) so the v1 consumer continues the stream. The
    /// session counts the fallback under `reason`.
    Demote {
        reason: InlineFallbackReason,
        held: Vec<RtpPacket>,
    },
}

/// A per-session consumer that runs inline on the socket owner.
///
/// Implementations must not block or await: every call runs inside the
/// socket's receive loop.
pub trait InlineMediaSink: Send + 'static {
    /// One packet that the session has already authenticated and accounted.
    /// On `Demote`, the current packet has *not* been consumed.
    fn on_packet(&mut self, packet: &InlineRtpPacket<'_>, now: Instant) -> InlineSinkVerdict;

    /// The earliest instant the sink needs service without a new packet.
    fn next_deadline(&mut self, now: Instant) -> Option<Instant>;

    /// The deadline returned by [`Self::next_deadline`] has passed.
    fn on_deadline(&mut self, now: Instant) -> InlineSinkVerdict;
}

/// Transport → session seam. The socket owner hands each authenticated,
/// source-accepted RTP media packet to the session's inline ingress instead
/// of publishing `RtpEvent::MediaReceived` on the transport broadcast.
pub(crate) trait InlineRtpIngress: Send + 'static {
    fn on_rtp(&mut self, packet: &InlineRtpPacket<'_>, now: Instant);
    fn next_deadline(&mut self, now: Instant) -> Option<Instant>;
    fn on_deadline(&mut self, now: Instant);

    /// One RTCP compound received on the RTP socket (multiplexed), after the
    /// transport's source check and SRTCP unprotect. Returns whether it was
    /// processed here; `false` (the default) sends it to the transport
    /// broadcast, as v1 does. Owner-side RTCP processing builds on this.
    fn on_rtcp(&mut self, _data: &[u8], _now: Instant) -> bool {
        false
    }
}

/// Single-use hand-off from a control-plane thread to the owner task.
///
/// The owner checks [`Self::is_dirty`] (one Acquire load) before each packet
/// and takes the value only when it changed. No lock is taken per packet.
pub(crate) struct HandoffSlot<T> {
    dirty: AtomicBool,
    claimed: AtomicBool,
    pending: parking_lot::Mutex<Option<T>>,
}

impl<T> HandoffSlot<T> {
    pub(crate) fn new() -> Self {
        Self {
            dirty: AtomicBool::new(false),
            claimed: AtomicBool::new(false),
            pending: parking_lot::Mutex::new(None),
        }
    }

    /// Offer the value once. A second offer returns it unchanged.
    pub(crate) fn offer(&self, value: T) -> Result<(), T> {
        if self.claimed.swap(true, Ordering::AcqRel) {
            return Err(value);
        }
        *self.pending.lock() = Some(value);
        self.dirty.store(true, Ordering::Release);
        Ok(())
    }

    pub(crate) fn is_claimed(&self) -> bool {
        self.claimed.load(Ordering::Acquire)
    }

    #[inline]
    pub(crate) fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Acquire)
    }

    pub(crate) fn take(&self) -> Option<T> {
        if !self.dirty.swap(false, Ordering::AcqRel) {
            return None;
        }
        self.pending.lock().take()
    }
}

/// Why a session did not (or no longer does) use the inline path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum InlineFallbackReason {
    /// The negotiated codec needs the async decoder.
    Codec,
    /// The consumer attached after packets were already broadcast.
    LateInstall,
    /// The transport or session refused the inline sink.
    Refused,
    /// A bridge or other packet subscriber needs `PacketReceived`.
    Demoted,
    /// A re-INVITE moved the dialog to a codec the inline path cannot decode.
    CodecChanged,
    /// A decoded-audio consumer could not be fused onto the owner (the
    /// dialog was stopped, demoted or rebound). The consumer's own task stays.
    ConsumerRefused,
    /// A fused consumer left the owner before the call ended.
    ConsumerDetached,
}

impl InlineFallbackReason {
    /// A stable snake-case label, for metrics exported by the application.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codec => "codec",
            Self::LateInstall => "late_install",
            Self::Refused => "refused",
            Self::Demoted => "demoted",
            Self::CodecChanged => "codec_changed",
            Self::ConsumerRefused => "consumer_refused",
            Self::ConsumerDetached => "consumer_detached",
        }
    }
}

static INLINE_PACKETS: AtomicU64 = AtomicU64::new(0);
static WAKEUPS_AVOIDED: AtomicU64 = AtomicU64::new(0);
static FALLBACKS: AtomicU64 = AtomicU64::new(0);
static INLINE_SINKS: AtomicU64 = AtomicU64::new(0);
static OWNER_TIMER_FIRES: AtomicU64 = AtomicU64::new(0);
static OWNER_TIMER_RESETS: AtomicU64 = AtomicU64::new(0);
static DRAIN_BUDGET_YIELDS: AtomicU64 = AtomicU64::new(0);
static OWNER_TIMER_REARMS_SKIPPED: AtomicU64 = AtomicU64::new(0);
static LANE_WAKEUPS: AtomicU64 = AtomicU64::new(0);
static LANE_DATAGRAMS: AtomicU64 = AtomicU64::new(0);
static LANE_SOCKETS: AtomicU64 = AtomicU64::new(0);
static LANE_BUDGET_YIELDS: AtomicU64 = AtomicU64::new(0);
static LANE_HOSTED_TASKS: AtomicU64 = AtomicU64::new(0);
static LANE_TIMER_FIRES: AtomicU64 = AtomicU64::new(0);
static LANE_TIMER_WAKES: AtomicU64 = AtomicU64::new(0);
static LANE_TIMER_ARMS: AtomicU64 = AtomicU64::new(0);
static TX_SYNC_SENDS: AtomicU64 = AtomicU64::new(0);
static TX_SYNC_DEFERRED: AtomicU64 = AtomicU64::new(0);
static TX_BATCHES: AtomicU64 = AtomicU64::new(0);
static TX_BATCHED_PACKETS: AtomicU64 = AtomicU64::new(0);
static TX_BATCH_FALLBACKS: AtomicU64 = AtomicU64::new(0);

/// Media executor totals for this process since start. Each field is
/// individually atomic; the set is not a transaction.
///
/// The counters live in process atomics, published in batches by their
/// owners, so the packet path never touches a shared cache line per packet.
/// Applications export them as they see fit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct InlineExecutorCounters {
    /// Inbound RTP packets processed inline by a sink.
    pub inline_packets: u64,
    /// Task wakeups the v1 graph would have spent on the inline work (the
    /// session receive task and the media handler: two per inline packet).
    pub wakeups_avoided: u64,
    /// Sessions that fell back to (or were demoted to) v1.
    pub fallbacks: u64,
    /// Inline sinks that became active.
    pub inline_sinks: u64,
    /// Owner timer expiries (every inline deadline of a call shares one
    /// timer per socket owner).
    pub owner_timer_fires: u64,
    /// Owner timer registrations with the runtime timer: only an earlier
    /// deadline, or a deadline after the timer fired, registers.
    pub owner_timer_resets: u64,
    /// Owner yields forced by the receive drain budget.
    pub drain_budget_yields: u64,
    /// Owner deadline changes that did not register the runtime timer (a
    /// later deadline extends the live entry in place; a cleared deadline
    /// leaves it).
    pub owner_timer_rearms_skipped: u64,
    /// Polls of a lane poller (one task wakeup each, however many sockets it
    /// served).
    pub lane_wakeups: u64,
    /// Datagrams received by sockets hosted on a lane poller.
    /// `lane_datagrams / lane_wakeups` is datagrams per wakeup.
    pub lane_datagrams: u64,
    /// Receive loops adopted by a lane poller.
    pub lane_sockets: u64,
    /// Lane poller yields forced by its per-wakeup budget.
    pub lane_budget_yields: u64,
    /// Non-socket futures (paced TX tasks) hosted on a lane poller.
    pub lane_hosted_tasks: u64,
    /// Expiries of a lane poller's one runtime timer (the lane clock).
    pub lane_timer_fires: u64,
    /// Lane-clock entries whose deadline passed and whose future was woken.
    /// `lane_timer_wakes / lane_timer_fires` is entries per expiry.
    pub lane_timer_wakes: u64,
    /// Deadlines filed in a lane clock (a new or moved deadline).
    pub lane_timer_arms: u64,
    /// RTP packets sent synchronously from the caller (`try_send_to` or
    /// `sendmmsg`) without the async send chain.
    pub tx_sync_sends: u64,
    /// Synchronous sends that handed the frame back to the async path before
    /// consuming anything (contended writer, socket not writable, non-UDP
    /// transport).
    pub tx_sync_deferred: u64,
    /// `sendmmsg` calls that carried more than one packet.
    pub tx_batches: u64,
    /// Packets that left in those `sendmmsg` calls.
    pub tx_batched_packets: u64,
    /// Batches (or batch remainders) sent packet by packet because
    /// `sendmmsg` is unavailable or returned short.
    pub tx_batch_fallbacks: u64,
}

/// The media executor totals since start.
pub fn inline_executor_counters() -> InlineExecutorCounters {
    InlineExecutorCounters {
        inline_packets: INLINE_PACKETS.load(Ordering::Relaxed),
        wakeups_avoided: WAKEUPS_AVOIDED.load(Ordering::Relaxed),
        fallbacks: FALLBACKS.load(Ordering::Relaxed),
        inline_sinks: INLINE_SINKS.load(Ordering::Relaxed),
        owner_timer_fires: OWNER_TIMER_FIRES.load(Ordering::Relaxed),
        owner_timer_resets: OWNER_TIMER_RESETS.load(Ordering::Relaxed),
        drain_budget_yields: DRAIN_BUDGET_YIELDS.load(Ordering::Relaxed),
        owner_timer_rearms_skipped: OWNER_TIMER_REARMS_SKIPPED.load(Ordering::Relaxed),
        lane_wakeups: LANE_WAKEUPS.load(Ordering::Relaxed),
        lane_datagrams: LANE_DATAGRAMS.load(Ordering::Relaxed),
        lane_sockets: LANE_SOCKETS.load(Ordering::Relaxed),
        lane_budget_yields: LANE_BUDGET_YIELDS.load(Ordering::Relaxed),
        lane_hosted_tasks: LANE_HOSTED_TASKS.load(Ordering::Relaxed),
        lane_timer_fires: LANE_TIMER_FIRES.load(Ordering::Relaxed),
        lane_timer_wakes: LANE_TIMER_WAKES.load(Ordering::Relaxed),
        lane_timer_arms: LANE_TIMER_ARMS.load(Ordering::Relaxed),
        tx_sync_sends: TX_SYNC_SENDS.load(Ordering::Relaxed),
        tx_sync_deferred: TX_SYNC_DEFERRED.load(Ordering::Relaxed),
        tx_batches: TX_BATCHES.load(Ordering::Relaxed),
        tx_batched_packets: TX_BATCHED_PACKETS.load(Ordering::Relaxed),
        tx_batch_fallbacks: TX_BATCH_FALLBACKS.load(Ordering::Relaxed),
    }
}

/// Count one fallback. Control-plane rate (at most a few per session).
pub fn record_inline_fallback(reason: InlineFallbackReason) {
    FALLBACKS.fetch_add(1, Ordering::Relaxed);
    tracing::debug!(
        reason = reason.as_str(),
        "media session left the inline path"
    );
}

pub(crate) fn record_inline_sink_active() {
    INLINE_SINKS.fetch_add(1, Ordering::Relaxed);
}

/// Receives a socket owner takes back-to-back before it yields to the
/// runtime. The inline receive path drains with `try_recv_from`, which does
/// not spend Tokio's cooperative budget, so this bound is what keeps one busy
/// socket from starving the other tasks on its lane, and lets an owner
/// deadline that came due during a burst run.
pub const INLINE_DRAIN_BUDGET: usize = 50;

/// How often owner-local counters publish to the process totals.
const FLUSH_EVERY: u64 = 64;

/// Owner-local timer and drain counters, published in batches.
#[derive(Default)]
pub(crate) struct InlineOwnerCounters {
    pending_fires: u64,
    pending_resets: u64,
    pending_yields: u64,
    pending_skipped: u64,
}

impl InlineOwnerCounters {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn maybe_flush(&mut self) {
        if self.pending_fires + self.pending_resets + self.pending_yields + self.pending_skipped
            >= FLUSH_EVERY
        {
            self.flush();
        }
    }

    #[inline]
    pub(crate) fn fire(&mut self) {
        self.pending_fires += 1;
        self.maybe_flush();
    }

    #[inline]
    pub(crate) fn reset(&mut self) {
        self.pending_resets += 1;
        self.maybe_flush();
    }

    #[inline]
    pub(crate) fn budget_yield(&mut self) {
        self.pending_yields += 1;
        self.maybe_flush();
    }

    /// Account one [`OwnerTimer::arm`] outcome.
    #[inline]
    pub(crate) fn armed(&mut self, outcome: ArmOutcome) {
        match outcome {
            ArmOutcome::Unchanged => {}
            ArmOutcome::Registered => self.reset(),
            ArmOutcome::Extended | ArmOutcome::Cleared => {
                self.pending_skipped += 1;
                self.maybe_flush();
            }
        }
    }

    pub(crate) fn flush(&mut self) {
        publish(&OWNER_TIMER_FIRES, &mut self.pending_fires);
        publish(&OWNER_TIMER_RESETS, &mut self.pending_resets);
        publish(&DRAIN_BUDGET_YIELDS, &mut self.pending_yields);
        publish(&OWNER_TIMER_REARMS_SKIPPED, &mut self.pending_skipped);
    }
}

impl Drop for InlineOwnerCounters {
    fn drop(&mut self) {
        self.flush();
    }
}

#[inline]
fn publish(total: &AtomicU64, pending: &mut u64) {
    let value = std::mem::take(pending);
    if value > 0 {
        total.fetch_add(value, Ordering::Relaxed);
    }
}

/// What one [`OwnerTimer::arm`] did with the runtime timer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArmOutcome {
    /// Same deadline as before: nothing to do.
    Unchanged,
    /// The runtime timer was (re-)registered with the timer driver: the
    /// deadline moved earlier than the live entry, or there was no live
    /// entry (first arm, or the previous deadline fired).
    Registered,
    /// The deadline moved later while the entry is still live. Tokio extends
    /// the live entry in place (`Sleep::reset` on a later instant re-files it
    /// lazily, without waking the owner). No re-registration.
    Extended,
    /// Nothing is pending any more. The live entry is left alone; it is not
    /// polled, and the next deadline extends or replaces it.
    Cleared,
}

/// The single runtime timer a socket owner arms.
///
/// Every inline deadline source of the owner's call (the reorder hold, the
/// decoded pacing queue, a consumer's playout clock) reports through
/// `InlineRtpIngress::next_deadline`, which returns their minimum. This timer
/// waits for that minimum.
///
/// The timer is re-registered with the timer driver only when the earliest
/// deadline moves *earlier* than the live entry, or when there is no live
/// entry (it fired). A later deadline keeps the registration and extends it
/// in place; an expiry is followed by serving every due source and exactly
/// one re-arm.
///
/// Deadlines are kept exact rather than rounded onto a private 1 ms grid:
/// Tokio's wheel already fires on 1 ms ticks, and a second, unaligned grid
/// would compound to almost 2 ms of lateness.
pub(crate) struct OwnerTimer {
    sleep: Option<Pin<Box<crate::media_clock::MediaSleep>>>,
    /// The deadline the owner waits for.
    armed: Option<Instant>,
    /// The instant the runtime entry was last set to and not yet observed to
    /// fire. `None` when there is no entry or it fired.
    live: Option<Instant>,
}

impl OwnerTimer {
    pub(crate) fn new() -> Self {
        Self {
            sleep: None,
            armed: None,
            live: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn armed(&self) -> Option<Instant> {
        self.armed
    }

    /// Wait for `deadline` (`None`: nothing pending). `now` decides whether
    /// the live entry can still be extended: an entry whose instant has
    /// passed may already have fired, so it is conservatively re-registered.
    pub(crate) fn arm(&mut self, deadline: Option<Instant>, now: Instant) -> ArmOutcome {
        if deadline == self.armed {
            return ArmOutcome::Unchanged;
        }
        self.armed = deadline;
        let Some(deadline) = deadline else {
            return ArmOutcome::Cleared;
        };
        let when = tokio::time::Instant::from_std(deadline);
        let extends = self.live.is_some_and(|live| live > now && deadline >= live);
        match self.sleep.as_mut() {
            Some(sleep) => sleep.as_mut().reset(when),
            None => self.sleep = Some(Box::pin(crate::media_clock::sleep_until(when))),
        }
        self.live = Some(deadline);
        if extends {
            ArmOutcome::Extended
        } else {
            ArmOutcome::Registered
        }
    }

    /// The runtime timer, only while a deadline is armed.
    pub(crate) fn armed_sleep(&mut self) -> Option<Pin<&mut crate::media_clock::MediaSleep>> {
        if self.armed.is_none() {
            return None;
        }
        self.sleep.as_mut().map(|sleep| sleep.as_mut())
    }

    /// The armed deadline has passed (fired, or found due during a drain).
    /// The next [`Self::arm`] re-registers even if it repeats the instant: a
    /// fired sleep must be reset before it is polled again.
    pub(crate) fn fired(&mut self) {
        self.armed = None;
        self.live = None;
    }

    pub(crate) fn is_due(&self, now: Instant) -> bool {
        self.armed.is_some_and(|deadline| deadline <= now)
    }
}

/// Wakeups the v1 graph spends per inbound packet between the socket task
/// and the media handler that the inline path does not.
pub const WAKEUPS_AVOIDED_PER_INLINE_PACKET: u64 = 2;

/// Owner-local packet counters, published in batches so the packet path does
/// not touch a shared cache line per packet.
#[derive(Default)]
pub(crate) struct InlinePacketCounters {
    pending_packets: u64,
}

impl InlinePacketCounters {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub(crate) fn packet(&mut self) {
        self.pending_packets += 1;
        if self.pending_packets >= FLUSH_EVERY {
            self.flush();
        }
    }

    pub(crate) fn flush(&mut self) {
        let packets = std::mem::take(&mut self.pending_packets);
        if packets == 0 {
            return;
        }
        INLINE_PACKETS.fetch_add(packets, Ordering::Relaxed);
        WAKEUPS_AVOIDED.fetch_add(
            packets * WAKEUPS_AVOIDED_PER_INLINE_PACKET,
            Ordering::Relaxed,
        );
    }
}

impl Drop for InlinePacketCounters {
    fn drop(&mut self) {
        self.flush();
    }
}

/// Add wakeups avoided by inline work outside rtp-core (a consumer fused onto
/// the socket owner). Callers batch: once per many frames.
pub fn record_wakeups_avoided(wakeups: u64) {
    WAKEUPS_AVOIDED.fetch_add(wakeups, Ordering::Relaxed);
}

/// Lane-poller counters, kept by the poller task and published in batches.
#[derive(Default)]
pub(crate) struct LanePollerCounters {
    pending_wakeups: u64,
    pending_datagrams: u64,
    pending_yields: u64,
}

impl LanePollerCounters {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// One poll of the lane task that received `datagrams` in total.
    #[inline]
    pub(crate) fn wakeup(&mut self, datagrams: u64, budget_yield: bool) {
        self.pending_wakeups += 1;
        self.pending_datagrams += datagrams;
        self.pending_yields += u64::from(budget_yield);
        if self.pending_wakeups >= FLUSH_EVERY {
            self.flush();
        }
    }

    pub(crate) fn flush(&mut self) {
        publish(&LANE_WAKEUPS, &mut self.pending_wakeups);
        publish(&LANE_DATAGRAMS, &mut self.pending_datagrams);
        publish(&LANE_BUDGET_YIELDS, &mut self.pending_yields);
    }
}

impl Drop for LanePollerCounters {
    fn drop(&mut self) {
        self.flush();
    }
}

/// Count one receive loop adopted by a lane poller. Control-plane rate.
pub(crate) fn record_lane_socket() {
    LANE_SOCKETS.fetch_add(1, Ordering::Relaxed);
}

/// Count one non-socket future hosted on a lane poller. Control-plane rate.
pub(crate) fn record_lane_hosted_task() {
    LANE_HOSTED_TASKS.fetch_add(1, Ordering::Relaxed);
}

/// Lane-clock counters, kept by the poller task and published in batches.
#[derive(Default)]
pub(crate) struct LaneTimerCounters {
    pending_fires: u64,
    pending_wakes: u64,
    pending_arms: u64,
}

impl LaneTimerCounters {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// One poll of the lane clock: whether its runtime timer had expired, how
    /// many entries it woke and how many deadlines were filed since the
    /// previous poll.
    #[inline]
    pub(crate) fn poll(&mut self, fired: bool, wakes: u64, arms: u64) {
        self.pending_fires += u64::from(fired);
        self.pending_wakes += wakes;
        self.pending_arms += arms;
        if self.pending_fires + self.pending_wakes + self.pending_arms >= FLUSH_EVERY {
            self.flush();
        }
    }

    pub(crate) fn flush(&mut self) {
        publish(&LANE_TIMER_FIRES, &mut self.pending_fires);
        publish(&LANE_TIMER_WAKES, &mut self.pending_wakes);
        publish(&LANE_TIMER_ARMS, &mut self.pending_arms);
    }
}

impl Drop for LaneTimerCounters {
    fn drop(&mut self) {
        self.flush();
    }
}

/// Synchronous TX counters owned by one writer (one call), published in
/// batches so the per-frame path touches no shared cache line. Fed from
/// `RtpSendNowReport`s by the caller of `RtpSendHandle::try_send_now`.
#[derive(Debug, Default)]
pub struct TxPathCounters {
    pending_sync: u64,
    pending_deferred: u64,
    pending_batches: u64,
    pending_batched: u64,
    pending_fallbacks: u64,
}

impl TxPathCounters {
    /// Counters with nothing pending.
    pub fn new() -> Self {
        Self::default()
    }

    fn maybe_flush(&mut self) {
        if self.pending_sync + self.pending_deferred + self.pending_batches + self.pending_fallbacks
            >= FLUSH_EVERY
        {
            self.flush();
        }
    }

    /// `packets` left synchronously.
    pub fn sync_sent(&mut self, packets: u64) {
        self.pending_sync += packets;
        self.maybe_flush();
    }

    /// One frame went back to the async path before consuming anything.
    pub fn deferred(&mut self) {
        self.pending_deferred += 1;
        self.maybe_flush();
    }

    /// One `sendmmsg` carried `packets` (more than one).
    pub fn batch(&mut self, packets: u64) {
        self.pending_batches += 1;
        self.pending_batched += packets;
        self.maybe_flush();
    }

    /// Part of a batch went packet by packet.
    pub fn batch_fallback(&mut self) {
        self.pending_fallbacks += 1;
        self.maybe_flush();
    }

    /// Publish everything pending to the process totals.
    pub fn flush(&mut self) {
        publish(&TX_SYNC_SENDS, &mut self.pending_sync);
        publish(&TX_SYNC_DEFERRED, &mut self.pending_deferred);
        publish(&TX_BATCHES, &mut self.pending_batches);
        publish(&TX_BATCHED_PACKETS, &mut self.pending_batched);
        publish(&TX_BATCH_FALLBACKS, &mut self.pending_fallbacks);
    }
}

impl Drop for TxPathCounters {
    fn drop(&mut self) {
        self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_v1_with_every_switch_off_and_v2_turns_every_switch_on() {
        let v1 = MediaExecutorConfig::default();
        assert_eq!(v1, MediaExecutorConfig::v1());
        assert_eq!(v1.mode, MediaExecutorMode::V1);
        assert!(!v1.lane_poller_active());
        assert!(!v1.lane_timer_active());
        assert!(!v1.lane_tx_active());
        assert!(!v1.tx_batch_active());
        assert!(!v1.fast_ssrc_hash_active());

        let v2 = MediaExecutorConfig::v2();
        assert_eq!(v2.mode, MediaExecutorMode::V2);
        assert!(v2.lane_poller_active());
        assert!(v2.lane_timer_active());
        assert!(v2.lane_tx_active());
        assert!(v2.tx_batch_active());
        assert!(v2.fast_ssrc_hash_active());
        assert!(v2.validate().is_ok());
    }

    #[test]
    fn switches_never_apply_under_v1_and_dependents_follow_the_lane_poller() {
        let v1_with_switches = MediaExecutorConfig::v2().with_mode(MediaExecutorMode::V1);
        assert!(!v1_with_switches.lane_poller_active(), "never under v1");
        assert!(!v1_with_switches.lane_timer_active(), "never under v1");
        assert!(!v1_with_switches.tx_batch_active(), "never under v1");
        assert!(!v1_with_switches.fast_ssrc_hash_active(), "never under v1");

        let no_poller = MediaExecutorConfig::v2().with_lane_poller(false);
        assert!(no_poller.validate().is_ok());
        assert!(
            !no_poller.lane_timer_active(),
            "the lane clock needs the poller"
        );
        assert!(
            !no_poller.lane_tx_active(),
            "lane-hosted TX needs the poller"
        );
        assert!(no_poller.tx_batch_active());
        assert!(no_poller.fast_ssrc_hash_active());

        let one_off = MediaExecutorConfig::v2()
            .with_lane_timer(false)
            .with_lane_tx(false)
            .with_tx_batch(false)
            .with_fast_ssrc_hash(false);
        assert!(one_off.lane_poller_active());
        assert!(!one_off.lane_timer_active());
        assert!(!one_off.lane_tx_active());
        assert!(!one_off.tx_batch_active());
        assert!(!one_off.fast_ssrc_hash_active());
    }

    #[test]
    fn mode_names_parse_and_reject_unknown_values() {
        assert_eq!("v1".parse(), Ok(MediaExecutorMode::V1));
        assert_eq!(" V2 ".parse(), Ok(MediaExecutorMode::V2));
        for bad in ["", "2", "v3", "on", "legacy"] {
            assert!(bad.parse::<MediaExecutorMode>().is_err(), "{bad}");
        }
        assert_eq!(MediaExecutorMode::V2.to_string(), "v2");
    }

    #[test]
    fn install_accepts_an_equal_configuration_and_rejects_a_different_one() {
        let slot = OnceLock::new();
        assert_eq!(install_into(&slot, MediaExecutorConfig::v2()), Ok(()));
        assert_eq!(install_into(&slot, MediaExecutorConfig::v2()), Ok(()));
        let error = install_into(&slot, MediaExecutorConfig::v1()).unwrap_err();
        assert_eq!(
            error,
            MediaExecutorConfigError::AlreadyConfigured {
                active: Box::new(MediaExecutorConfig::v2())
            }
        );
        let error = install_into(&slot, MediaExecutorConfig::v2().with_tx_batch(false));
        assert!(error.is_err(), "one switch differs");
    }

    #[test]
    fn thread_override_is_scoped_and_restored() {
        let outer = override_media_executor_config_on_this_thread(MediaExecutorConfig::v2());
        assert_eq!(media_executor_mode(), MediaExecutorMode::V2);
        assert!(lane_poller_enabled() && lane_timer_enabled() && lane_tx_enabled());
        assert!(tx_batch_enabled() && fast_ssrc_hash_enabled());
        {
            let _inner = override_media_executor_config_on_this_thread(
                MediaExecutorConfig::v2().with_lane_poller(false),
            );
            assert!(!lane_poller_enabled());
            assert!(!lane_timer_enabled());
            assert!(tx_batch_enabled());
            let _v1 = override_media_executor_config_on_this_thread(MediaExecutorConfig::v1());
            assert_eq!(media_executor_mode(), MediaExecutorMode::V1);
            assert!(!tx_batch_enabled());
        }
        assert_eq!(media_executor_config(), MediaExecutorConfig::v2());
        drop(outer);
        assert!(!THREAD_HAS_OVERRIDE.with(Cell::get));
        assert!(THREAD_CONFIG_OVERRIDE.with(|slot| slot.borrow().is_none()));
    }

    #[test]
    fn handoff_slot_is_single_use_and_lock_free_until_offered() {
        let slot = HandoffSlot::new();
        assert!(!slot.is_dirty());
        assert!(!slot.is_claimed());
        assert_eq!(slot.take(), None::<u8>);
        assert_eq!(slot.offer(7), Ok(()));
        assert_eq!(slot.offer(8), Err(8));
        assert!(slot.is_dirty());
        assert!(slot.is_claimed());
        assert_eq!(slot.take(), Some(7));
        assert!(!slot.is_dirty());
        assert_eq!(slot.take(), None);
    }

    #[test]
    fn inline_packet_rebuilds_the_v1_minimal_packet() {
        let packet = InlineRtpPacket {
            payload_type: 0,
            sequence_number: 9,
            timestamp: 1440,
            marker: true,
            ssrc: 0x0102_0304,
            padding_size: 3,
            payload: &[1, 2, 3],
            source: "127.0.0.1:4000".parse().unwrap(),
        };
        let owned = packet.to_packet();
        assert_eq!(owned.header.sequence_number, 9);
        assert!(owned.header.padding && owned.header.marker && !owned.header.extension);
        assert!(owned.header.csrc.is_empty() && owned.header.extensions.is_none());
        assert_eq!(&owned.payload[..], &[1, 2, 3]);
        assert_eq!(owned.padding_size, 3);
        assert_eq!(owned.size(), packet.accounted_size());
    }

    #[test]
    fn fallback_reasons_have_stable_labels() {
        assert_eq!(InlineFallbackReason::LateInstall.as_str(), "late_install");
        assert_eq!(
            InlineFallbackReason::ConsumerRefused.as_str(),
            "consumer_refused"
        );
        assert_eq!(
            InlineFallbackReason::ConsumerDetached.as_str(),
            "consumer_detached"
        );
    }

    #[test]
    fn owner_and_packet_counters_flush_in_batches_and_on_drop() {
        let before = inline_executor_counters();
        let mut packets = InlinePacketCounters::new();
        for _ in 0..(FLUSH_EVERY + 3) {
            packets.packet();
        }
        drop(packets);
        let mut owner = InlineOwnerCounters::new();
        owner.fire();
        owner.reset();
        owner.reset();
        owner.budget_yield();
        drop(owner);
        let after = inline_executor_counters();
        // Other tests may run concurrently; this one contributed at least its own.
        assert!(after.inline_packets >= before.inline_packets + FLUSH_EVERY + 3);
        assert!(
            after.wakeups_avoided
                >= before.wakeups_avoided + WAKEUPS_AVOIDED_PER_INLINE_PACKET * (FLUSH_EVERY + 3)
        );
        assert!(after.owner_timer_fires > before.owner_timer_fires);
        assert!(after.owner_timer_resets >= before.owner_timer_resets + 2);
        assert!(after.drain_budget_yields > before.drain_budget_yields);
    }

    #[test]
    fn owner_counters_split_reregistrations_from_skipped_rearms() {
        let before = inline_executor_counters();
        let mut owner = InlineOwnerCounters::new();
        owner.armed(ArmOutcome::Unchanged);
        owner.armed(ArmOutcome::Registered);
        owner.armed(ArmOutcome::Extended);
        owner.armed(ArmOutcome::Cleared);
        drop(owner);
        let after = inline_executor_counters();
        assert!(after.owner_timer_resets > before.owner_timer_resets);
        assert!(after.owner_timer_rearms_skipped >= before.owner_timer_rearms_skipped + 2);
    }

    #[test]
    fn lane_and_tx_counters_flush_in_batches_and_on_drop() {
        let before = inline_executor_counters();
        let mut lane = LanePollerCounters::new();
        lane.wakeup(3, false);
        lane.wakeup(0, true);
        drop(lane);
        let mut timer = LaneTimerCounters::new();
        timer.poll(true, 2, 1);
        drop(timer);
        let mut tx = TxPathCounters::new();
        tx.sync_sent(FLUSH_EVERY);
        let flushed = inline_executor_counters();
        assert!(flushed.tx_sync_sends >= before.tx_sync_sends + FLUSH_EVERY);
        tx.deferred();
        tx.batch(4);
        tx.batch_fallback();
        drop(tx);
        record_lane_socket();
        record_lane_hosted_task();
        let after = inline_executor_counters();
        assert!(after.lane_wakeups >= before.lane_wakeups + 2);
        assert!(after.lane_datagrams >= before.lane_datagrams + 3);
        assert!(after.lane_budget_yields > before.lane_budget_yields);
        assert!(after.lane_timer_fires > before.lane_timer_fires);
        assert!(after.lane_timer_wakes >= before.lane_timer_wakes + 2);
        assert!(after.lane_timer_arms > before.lane_timer_arms);
        assert!(after.tx_sync_deferred > before.tx_sync_deferred);
        assert!(after.tx_batches > before.tx_batches);
        assert!(after.tx_batched_packets >= before.tx_batched_packets + 4);
        assert!(after.tx_batch_fallbacks > before.tx_batch_fallbacks);
        assert!(after.lane_sockets > before.lane_sockets);
        assert!(after.lane_hosted_tasks > before.lane_hosted_tasks);
    }

    #[tokio::test]
    async fn owner_timer_registers_only_when_the_deadline_moves_earlier_or_after_a_fire() {
        use std::time::Duration;
        let mut timer = OwnerTimer::new();
        assert!(timer.armed_sleep().is_none());
        let start = Instant::now();
        let first = start + Duration::from_millis(20);
        assert_eq!(
            timer.arm(Some(first), start),
            ArmOutcome::Registered,
            "first arm registers"
        );
        assert_eq!(timer.arm(Some(first), start), ArmOutcome::Unchanged);
        assert!(!timer.is_due(start));
        assert!(timer.is_due(first));
        // A later deadline keeps the live registration (extended in place).
        let later = start + Duration::from_millis(40);
        assert_eq!(timer.arm(Some(later), start), ArmOutcome::Extended);
        assert_eq!(timer.armed(), Some(later));
        // An earlier one re-registers.
        let earlier = start + Duration::from_millis(5);
        assert_eq!(timer.arm(Some(earlier), start), ArmOutcome::Registered);
        assert_eq!(timer.armed(), Some(earlier));
        timer.armed_sleep().expect("armed").await;
        assert!(Instant::now() >= earlier, "never fires early");
        timer.fired();
        assert!(
            timer.armed_sleep().is_none(),
            "a fired timer is not polled again"
        );
        let now = Instant::now();
        assert_eq!(
            timer.arm(Some(earlier), now),
            ArmOutcome::Registered,
            "a fired timer re-registers even for the same instant"
        );
        assert_eq!(
            timer.arm(None, now),
            ArmOutcome::Cleared,
            "clearing keeps the entry"
        );
        assert!(timer.armed_sleep().is_none());
        // After a clear, a later deadline extends the still-live entry only
        // while that entry's instant has not passed.
        let future = now + Duration::from_millis(30);
        let mut fresh = OwnerTimer::new();
        assert_eq!(fresh.arm(Some(future), now), ArmOutcome::Registered);
        assert_eq!(fresh.arm(None, now), ArmOutcome::Cleared);
        assert_eq!(
            fresh.arm(Some(future + Duration::from_millis(1)), now),
            ArmOutcome::Extended
        );
        assert_eq!(
            fresh.arm(
                Some(future + Duration::from_millis(2)),
                future + Duration::from_millis(1)
            ),
            ArmOutcome::Registered,
            "an entry whose instant passed may have fired: re-register"
        );
    }

    #[tokio::test]
    async fn an_extended_owner_timer_never_fires_before_the_later_deadline() {
        use std::time::Duration;
        let mut timer = OwnerTimer::new();
        let start = Instant::now();
        let first = start + Duration::from_millis(60);
        let later = start + Duration::from_millis(120);
        assert_eq!(timer.arm(Some(first), start), ArmOutcome::Registered);
        // Poll once so the entry is registered with a waker, as in the loop.
        let polled =
            tokio::time::timeout(Duration::from_millis(1), timer.armed_sleep().unwrap()).await;
        assert!(polled.is_err());
        assert_eq!(timer.arm(Some(later), Instant::now()), ArmOutcome::Extended);
        timer.armed_sleep().expect("armed").await;
        assert!(
            Instant::now() >= later,
            "the extension holds: no early expiry at {first:?}"
        );
    }
}
