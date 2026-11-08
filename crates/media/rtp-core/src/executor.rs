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
//! - [`MediaExecutorConfig::inline_decode`]: the media layer installs an
//!   [`InlineMediaSink`] on each dialog it can decode synchronously (G.711),
//!   so reorder, decode and pacing run on the socket owner too, instead of on
//!   the dialog's own receive task.
//! - [`MediaExecutorConfig::pcm_pool`]: decoded and concealed PCM buffers on
//!   the media path are reused through a small per-thread pool instead of
//!   being allocated per frame.
//! - [`MediaExecutorConfig::lane_tick`]: a lane poller on a dedicated
//!   current-thread media lane serves its sockets and deadlines on a tick
//!   grid instead of waking for every readiness event (see
//!   [`crate::lane_tick`]). Needs the lane poller.
//! - [`MediaExecutorConfig::lane_timer_wheel`]: lane clocks keep their
//!   deadlines in a timing wheel (O(1) file, cancel and re-arm) instead of a
//!   binary heap. Needs the lane clock.
//! - [`MediaExecutorConfig::lane_pass_clock`]: a lane poller pass reads the
//!   clock once and the futures it polls read that value through
//!   [`crate::media_clock::now`]. Needs the lane poller.
//! - [`MediaExecutorConfig::owner_srtp`]: the receive loop checks the
//!   transport's inbound SRTP context out and uses it without the context
//!   lock, which it takes again only after a rekey, rollback or removal.
//! - [`MediaExecutorConfig::owner_rtcp`]: RTCP multiplexed on the RTP socket
//!   is processed by the session's RTCP handling on the socket owner instead
//!   of crossing the transport broadcast to the session task, and periodic
//!   reports leave with a synchronous send (from a future hosted on the
//!   socket's lane poller when the lane poller is on).
//! - [`MediaExecutorConfig::owner_receive_stats`]: the socket owner publishes
//!   the per-packet receive statistics through a single-writer seqlock
//!   instead of taking the session statistics mutex per packet.
//! - [`MediaExecutorConfig::counter_pool_cleanup`]: the per-thread PCM pool
//!   keeps up to 1024 small buffers instead of 32, and a lane poller on a
//!   current-thread runtime publishes the per-event counters its passes add
//!   (PCM pool misses, owner RTCP) once per batch of passes.
//! - [`MediaExecutorConfig::hot_state`]: per-call state the socket owner
//!   touches for every packet is packed onto fewer cache lines (the SRTP
//!   context's per-direction keys and rollover state, the receive loop's
//!   control entry in its lane's slab, the synchronous TX path's
//!   destination), the receive loop checks its control-plane fences through
//!   one doorbell line, and lane clocks prefetch the state of the futures
//!   they are about to wake (see [`crate::hot_state`]).
//!
//! Every decision is read when an object is built (a transport starts, a
//! session or SRTP context is created, a poller starts), never per packet.

use std::cell::Cell;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use bytes::Bytes;

use crate::packet::{RtpHeader, RtpPacket};

// ---- Lane tick (wakeup and timer coalescing) ----
pub use crate::lane_tick::LaneTick;
// ---- End lane tick ----

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
    // ---- Media-layer inline receive (decode and PCM reuse on the owner) ----
    /// The media layer decodes, reorders and paces a G.711 dialog's inbound
    /// audio inline on the socket owner, through the session's
    /// [`InlineMediaSink`], instead of on the dialog's receive task.
    pub inline_decode: bool,
    /// Decoded and concealed PCM buffers on the media path are reused through
    /// a small per-thread pool instead of being allocated per frame.
    pub pcm_pool: bool,
    // ---- End media-layer inline receive ----
    // ---- Lane tick (wakeup and timer coalescing) ----
    /// Lane pollers on [`MediaLanes::Dedicated`] lanes run on a tick of this
    /// quantum: after a pass the lane thread sleeps to the next tick boundary
    /// and serves every socket and deadline that became ready meanwhile in
    /// one pass (see [`crate::lane_tick`]). `None`: every readiness event
    /// wakes the lane. Needs [`Self::lane_poller`]; on a lane that is not a
    /// dedicated current-thread lane it reads as off for that lane.
    ///
    /// [`MediaLanes::Dedicated`]: crate::task_runtime::MediaLanes::Dedicated
    pub lane_tick: Option<LaneTick>,
    // ---- End lane tick ----
    // ---- Lane timing wheel and pass clock ----
    /// Lane clocks keep their deadlines in a single-level timing wheel (O(1)
    /// file, cancel and re-arm, no allocation; far deadlines wait in an
    /// overflow heap) instead of a binary heap. Firing order and times are
    /// the heap's. Needs [`Self::lane_timer`]; without it, it reads as off.
    pub lane_timer_wheel: bool,
    /// A lane poller pass reads the clock once and the futures it polls read
    /// that value through [`crate::media_clock::now`] and
    /// [`crate::media_clock::now_std`] instead of reading the clock
    /// themselves. A pass reading is never later than the clock (deadline
    /// checks can be late by a bounded amount of lane work, never early).
    /// Needs [`Self::lane_poller`]; without it, it reads as off.
    pub lane_pass_clock: bool,
    // ---- End lane timing wheel and pass clock ----
    /// The receive loop owns the transport's inbound SRTP/SRTCP context and
    /// uses it without the context lock. A transport with a separate RTCP
    /// socket (whose own receive task also unprotects SRTCP) keeps the lock.
    pub owner_srtp: bool,
    /// Multiplexed inbound RTCP is processed on the socket owner (same
    /// stranger filter and handling as the session task), and periodic
    /// reports are sent synchronously; with [`Self::lane_poller`] the report
    /// task is hosted on the socket's lane poller. RTCP on a separate RTCP
    /// socket keeps its own receive task.
    pub owner_rtcp: bool,
    /// The socket owner, the only writer of a v2 session's per-packet receive
    /// statistics, publishes them without the session statistics mutex
    /// (a single-writer seqlock that readers overlay).
    pub owner_receive_stats: bool,
    // ---- Counter and PCM pool cleanup ----
    /// The per-thread PCM pool (with [`Self::pcm_pool`]) keeps up to 1024
    /// small buffers instead of 32, so a lane pass whose arrivals and playout
    /// ticks do not interleave no longer allocates, and a lane poller on a
    /// current-thread runtime publishes the per-event counters added during
    /// its passes (PCM pool misses, owner RTCP) once per batch of passes
    /// instead of one shared atomic add per event (see
    /// [`add_lane_deferred`]).
    pub counter_pool_cleanup: bool,
    // ---- End counter and PCM pool cleanup ----
    // ---- Hot call state ----
    /// Per-call state the socket owner touches for every packet is packed
    /// onto fewer cache lines, without changing any output: the SRTP
    /// context's per-direction keys, keyed HMAC and active stream's
    /// rollover/replay state (three lines per direction), the receive loop's
    /// control entry (in its lane's slab on a dedicated current-thread lane),
    /// and the synchronous TX path's destination. The receive loop reads one
    /// doorbell line per datagram instead of the transport's control-plane
    /// atomics, which it re-reads only after a control-plane change. Lane
    /// clocks prefetch the state of the futures they are about to wake. See
    /// [`crate::hot_state`].
    pub hot_state: bool,
    // ---- End hot call state ----
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
            // Media-layer inline receive.
            inline_decode: false,
            pcm_pool: false,
            // Lane tick.
            lane_tick: None,
            // Lane timing wheel and pass clock.
            lane_timer_wheel: false,
            lane_pass_clock: false,
            owner_srtp: false,
            owner_rtcp: false,
            owner_receive_stats: false,
            // Counter and PCM pool cleanup.
            counter_pool_cleanup: false,
            // Hot call state.
            hot_state: false,
        }
    }

    /// The inline executor with every switch on, a 2 ms lane tick with grid
    /// timers, the lane timing wheel and the pass clock.
    pub fn v2() -> Self {
        Self {
            mode: MediaExecutorMode::V2,
            lane_poller: true,
            lane_timer: true,
            lane_tx: true,
            tx_batch: true,
            fast_ssrc_hash: true,
            // Media-layer inline receive.
            inline_decode: true,
            pcm_pool: true,
            // Lane tick.
            lane_tick: Some(LaneTick::new(Duration::from_millis(2)).with_grid_timers(true)),
            // Lane timing wheel and pass clock.
            lane_timer_wheel: true,
            lane_pass_clock: true,
            owner_srtp: true,
            owner_rtcp: true,
            owner_receive_stats: true,
            // Counter and PCM pool cleanup.
            counter_pool_cleanup: true,
            // Hot call state.
            hot_state: true,
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

    // ---- Media-layer inline receive ----

    /// Set [`Self::inline_decode`].
    pub fn with_inline_decode(mut self, enabled: bool) -> Self {
        self.inline_decode = enabled;
        self
    }

    /// Set [`Self::pcm_pool`].
    pub fn with_pcm_pool(mut self, enabled: bool) -> Self {
        self.pcm_pool = enabled;
        self
    }

    // ---- End media-layer inline receive ----

    // ---- Lane tick ----

    /// Set [`Self::lane_tick`].
    pub fn with_lane_tick(mut self, tick: Option<LaneTick>) -> Self {
        self.lane_tick = tick;
        self
    }

    // ---- End lane tick ----

    // ---- Lane timing wheel and pass clock ----

    /// Set [`Self::lane_timer_wheel`].
    pub fn with_lane_timer_wheel(mut self, enabled: bool) -> Self {
        self.lane_timer_wheel = enabled;
        self
    }

    /// Set [`Self::lane_pass_clock`].
    pub fn with_lane_pass_clock(mut self, enabled: bool) -> Self {
        self.lane_pass_clock = enabled;
        self
    }

    // ---- End lane timing wheel and pass clock ----

    /// Set [`Self::owner_srtp`].
    pub fn with_owner_srtp(mut self, enabled: bool) -> Self {
        self.owner_srtp = enabled;
        self
    }

    /// Set [`Self::owner_rtcp`].
    pub fn with_owner_rtcp(mut self, enabled: bool) -> Self {
        self.owner_rtcp = enabled;
        self
    }

    /// Set [`Self::owner_receive_stats`].
    pub fn with_owner_receive_stats(mut self, enabled: bool) -> Self {
        self.owner_receive_stats = enabled;
        self
    }

    // ---- Counter and PCM pool cleanup ----
    /// Set [`Self::counter_pool_cleanup`].
    pub fn with_counter_pool_cleanup(mut self, enabled: bool) -> Self {
        self.counter_pool_cleanup = enabled;
        self
    }
    // ---- End counter and PCM pool cleanup ----

    // ---- Hot call state ----

    /// Set [`Self::hot_state`].
    pub fn with_hot_state(mut self, enabled: bool) -> Self {
        self.hot_state = enabled;
        self
    }

    // ---- End hot call state ----

    /// Check the configuration without installing it.
    ///
    /// A switch whose prerequisite is off is not an error: it reads as off
    /// (switching the lane poller off is the documented way to keep receive
    /// loops on their own tasks). Rules for switches that need a specific
    /// media runtime layout are checked here.
    pub fn validate(&self) -> Result<(), MediaExecutorConfigError> {
        // ---- Lane tick ----
        if let Some(tick) = self.lane_tick {
            if !tick.has_valid_quantum() {
                return Err(MediaExecutorConfigError::InvalidLaneTickQuantum {
                    quantum: tick.quantum,
                });
            }
        }
        // ---- End lane tick ----
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

    // ---- Media-layer inline receive ----

    /// Whether the media layer offers an inline decoding sink to new
    /// sessions.
    pub fn inline_decode_active(&self) -> bool {
        self.is_v2() && self.inline_decode
    }

    /// Whether the media path reuses PCM buffers through the per-thread pool.
    pub fn pcm_pool_active(&self) -> bool {
        self.is_v2() && self.pcm_pool
    }

    // ---- End media-layer inline receive ----

    // ---- Lane tick ----

    /// The lane tick lane pollers use, when one is configured and the lane
    /// poller is active. A poller on a lane that is not a dedicated
    /// current-thread lane still keeps immediate wakeups.
    pub fn lane_tick_active(&self) -> Option<LaneTick> {
        self.lane_tick.filter(|_| self.lane_poller_active())
    }

    /// Whether a ticking poller files lane-clock deadlines on the tick grid
    /// ([`LaneTick::grid_timers`], which also needs the lane clock).
    pub fn lane_tick_grid_timers_active(&self) -> bool {
        self.lane_timer_active() && self.lane_tick_active().is_some_and(|tick| tick.grid_timers)
    }

    // ---- End lane tick ----

    // ---- Lane timing wheel and pass clock ----

    /// Whether lane clocks keep their deadlines in the timing wheel (needs
    /// the lane clock).
    pub fn lane_timer_wheel_active(&self) -> bool {
        self.lane_timer_active() && self.lane_timer_wheel
    }

    /// Whether lane poller passes serve one clock reading to the futures
    /// they poll (needs the lane poller).
    pub fn lane_pass_clock_active(&self) -> bool {
        self.lane_poller_active() && self.lane_pass_clock
    }

    // ---- End lane timing wheel and pass clock ----
    /// Whether receive loops started now own the inbound SRTP context.
    pub fn owner_srtp_active(&self) -> bool {
        self.is_v2() && self.owner_srtp
    }

    /// Whether sessions created now process multiplexed RTCP on the socket
    /// owner and send periodic reports synchronously.
    pub fn owner_rtcp_active(&self) -> bool {
        self.is_v2() && self.owner_rtcp
    }

    /// Whether the periodic report task of a session created now is hosted
    /// on its socket's lane poller.
    pub fn owner_rtcp_on_lane_active(&self) -> bool {
        self.owner_rtcp_active() && self.lane_poller_active()
    }

    /// Whether sessions created now publish receive statistics from the
    /// socket owner without the statistics mutex.
    pub fn owner_receive_stats_active(&self) -> bool {
        self.is_v2() && self.owner_receive_stats
    }

    // ---- Counter and PCM pool cleanup ----
    /// Whether the PCM pool keeps more small buffers and lane pollers batch
    /// the per-event counters of their passes.
    pub fn counter_pool_cleanup_active(&self) -> bool {
        self.is_v2() && self.counter_pool_cleanup
    }
    // ---- End counter and PCM pool cleanup ----

    // ---- Hot call state ----

    /// Whether objects built now pack and gate their per-call hot state.
    pub fn hot_state_active(&self) -> bool {
        self.is_v2() && self.hot_state
    }

    // ---- End hot call state ----
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
    // ---- Lane tick ----
    /// A [`LaneTick::quantum`] that does not divide the 20 ms media clock
    /// at the runtime's 1 ms timer resolution.
    #[error(
        "the lane tick quantum must be 1, 2, 4 or 5 ms (a divisor of the 20 ms media clock), got {quantum:?}"
    )]
    InvalidLaneTickQuantum {
        /// The rejected quantum.
        quantum: Duration,
    },
    // ---- End lane tick ----
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

// ---- Media-layer inline receive ----

/// Whether the media layer should offer an inline decoding sink to a session
/// it is setting up. Read when the media session starts.
pub fn inline_decode_enabled() -> bool {
    with_config(MediaExecutorConfig::inline_decode_active)
}

/// Whether the media path reuses PCM buffers through the per-thread pool.
///
/// Unlike the other switches this one is consulted per buffer (a
/// thread-local flag and one load of the installed configuration), because a
/// buffer can be returned by a different object than the one that took it.
pub fn pcm_pool_enabled() -> bool {
    with_config(MediaExecutorConfig::pcm_pool_active)
}

// ---- End media-layer inline receive ----

// ---- Lane tick ----

/// The lane tick in effect on this thread
/// ([`MediaExecutorConfig::lane_tick_active`]). Read when a lane poller or a
/// paced queue is built.
pub fn active_lane_tick() -> Option<LaneTick> {
    with_config(MediaExecutorConfig::lane_tick_active)
}

/// Whether a ticking poller started now files lane-clock deadlines on the
/// tick grid.
pub fn lane_tick_grid_timers_enabled() -> bool {
    with_config(MediaExecutorConfig::lane_tick_grid_timers_active)
}

// ---- End lane tick ----

// ---- Lane timing wheel and pass clock ----

/// Whether a lane clock built now keeps its deadlines in the timing wheel
/// ([`MediaExecutorConfig::lane_timer_wheel_active`]). Read when a lane
/// poller starts.
pub fn lane_timer_wheel_enabled() -> bool {
    with_config(MediaExecutorConfig::lane_timer_wheel_active)
}

/// Whether a lane poller started now serves one clock reading per pass
/// ([`MediaExecutorConfig::lane_pass_clock_active`]). Read when a lane
/// poller starts.
pub fn lane_pass_clock_enabled() -> bool {
    with_config(MediaExecutorConfig::lane_pass_clock_active)
}

// ---- End lane timing wheel and pass clock ----

/// Whether a receive loop started now owns its transport's inbound SRTP
/// context. Read when a transport starts.
pub fn owner_srtp_enabled() -> bool {
    with_config(MediaExecutorConfig::owner_srtp_active)
}

/// Whether a session created now processes multiplexed RTCP on the socket
/// owner and sends periodic reports synchronously. Read when a session is
/// created.
pub fn owner_rtcp_enabled() -> bool {
    with_config(MediaExecutorConfig::owner_rtcp_active)
}

/// Whether a session created now hosts its periodic report task on its
/// socket's lane poller. Read when a session starts.
pub fn owner_rtcp_on_lane_enabled() -> bool {
    with_config(MediaExecutorConfig::owner_rtcp_on_lane_active)
}

/// Whether a session created now publishes its receive statistics from the
/// socket owner without the statistics mutex. Read when a session is created.
pub fn owner_receive_stats_enabled() -> bool {
    with_config(MediaExecutorConfig::owner_receive_stats_active)
}

// ---- Counter and PCM pool cleanup ----

/// Whether the PCM pool keeps more small buffers and lane pollers batch the
/// per-event counters of their passes. Read by the pool when a buffer comes
/// back to a pool that already holds 32 (a buffer may be returned by another
/// object than the one that took it), and when a lane poller starts.
pub fn counter_pool_cleanup_enabled() -> bool {
    with_config(MediaExecutorConfig::counter_pool_cleanup_active)
}

// ---- End counter and PCM pool cleanup ----

// ---- Hot call state ----

/// Whether objects built now pack and gate their per-call hot state
/// ([`MediaExecutorConfig::hot_state_active`]). Read when an SRTP context,
/// transport, packet writer, session ingress or lane clock is built, never
/// per packet.
pub fn hot_state_enabled() -> bool {
    with_config(MediaExecutorConfig::hot_state_active)
}

// ---- End hot call state ----

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

    /// One RTCP compound received on the RTP socket (multiplexed) from
    /// `source`, after the transport's latch check and SRTCP unprotect.
    /// Returns whether it was processed here; `false` (the default) sends it
    /// to the transport broadcast, as v1 does.
    fn on_rtcp(&mut self, _data: &[u8], _source: SocketAddr, _now: Instant) -> bool {
        false
    }

    /// Hot call state: the address of the state the ingress's deadlines are
    /// served from (its inline sink), which the lane clock prefetches when
    /// the owner timer falls due; 0: none.
    fn prefetch_hint(&self) -> usize {
        0
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
static OWNER_SRTP_LOOPS: AtomicU64 = AtomicU64::new(0);
static OWNER_SRTP_HANDOFFS: AtomicU64 = AtomicU64::new(0);
static SRTP_LOCKS_AVOIDED: AtomicU64 = AtomicU64::new(0);
static OWNER_RTCP_INBOUND: AtomicU64 = AtomicU64::new(0);
static RTCP_SYNC_SENDS: AtomicU64 = AtomicU64::new(0);
static RTCP_SYNC_DEFERRED: AtomicU64 = AtomicU64::new(0);
static RTCP_LANE_TASKS: AtomicU64 = AtomicU64::new(0);
static OWNER_RECEIVE_STATS: AtomicU64 = AtomicU64::new(0);

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
    /// Receive loops that own their transport's inbound SRTP context.
    pub owner_srtp_loops: u64,
    /// Times an owning receive loop adopted or gave back the context (the
    /// first check-out, a rekey, a rollback, a removal).
    pub owner_srtp_handoffs: u64,
    /// Inbound SRTP/SRTCP datagrams authenticated without taking the
    /// context lock.
    pub srtp_locks_avoided: u64,
    /// Inbound RTCP compounds processed on the socket owner (each also
    /// counts one avoided session-task wakeup in `wakeups_avoided`).
    pub owner_rtcp_inbound: u64,
    /// Periodic RTCP reports written with `try_send_to` from the report
    /// task's poll.
    pub rtcp_sync_sends: u64,
    /// Periodic RTCP reports the socket did not take at once and that went
    /// through the awaited send (already protected, so each SRTCP index is
    /// still consumed once).
    pub rtcp_sync_deferred: u64,
    /// Periodic report tasks hosted on a lane poller.
    pub rtcp_lane_tasks: u64,
    /// Sessions whose socket owner publishes receive statistics without the
    /// statistics mutex.
    pub owner_receive_stats: u64,
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
        owner_srtp_loops: OWNER_SRTP_LOOPS.load(Ordering::Relaxed),
        owner_srtp_handoffs: OWNER_SRTP_HANDOFFS.load(Ordering::Relaxed),
        srtp_locks_avoided: SRTP_LOCKS_AVOIDED.load(Ordering::Relaxed),
        owner_rtcp_inbound: OWNER_RTCP_INBOUND.load(Ordering::Relaxed),
        rtcp_sync_sends: RTCP_SYNC_SENDS.load(Ordering::Relaxed),
        rtcp_sync_deferred: RTCP_SYNC_DEFERRED.load(Ordering::Relaxed),
        rtcp_lane_tasks: RTCP_LANE_TASKS.load(Ordering::Relaxed),
        owner_receive_stats: OWNER_RECEIVE_STATS.load(Ordering::Relaxed),
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
    /// Hot call state: the address the lane clock prefetches when this
    /// timer falls due (0: none); see [`Self::set_prefetch_hint`].
    hint: usize,
}

impl OwnerTimer {
    pub(crate) fn new() -> Self {
        Self {
            sleep: None,
            armed: None,
            live: None,
            hint: 0,
        }
    }

    /// Have the lane clock prefetch `address` (the owner's deadline
    /// consumer) when this timer falls due. Applied to the current sleep now
    /// and to a sleep created later; a sleep not filed in a lane clock
    /// ignores it.
    pub(crate) fn set_prefetch_hint(&mut self, address: usize) {
        if self.hint == address {
            return;
        }
        self.hint = address;
        if let Some(sleep) = self.sleep.as_mut() {
            sleep.as_mut().set_prefetch_hint(address);
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
            None => {
                let mut sleep = Box::pin(crate::media_clock::sleep_until(when));
                if self.hint != 0 {
                    sleep.as_mut().set_prefetch_hint(self.hint);
                }
                self.sleep = Some(sleep);
            }
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

// ---- Lane timing wheel and pass clock ----

static WHEEL_INSERTS: AtomicU64 = AtomicU64::new(0);
static WHEEL_FAR_INSERTS: AtomicU64 = AtomicU64::new(0);
static WHEEL_CANCELS: AtomicU64 = AtomicU64::new(0);
static WHEEL_CASCADES: AtomicU64 = AtomicU64::new(0);
static WHEEL_SLOT_SCANS: AtomicU64 = AtomicU64::new(0);
static PASS_CLOCK_CACHED: AtomicU64 = AtomicU64::new(0);
static PASS_CLOCK_FRESH: AtomicU64 = AtomicU64::new(0);
static PASS_CLOCK_STALE_SAMPLES: AtomicU64 = AtomicU64::new(0);
static PASS_CLOCK_STALE_SUM_NS: AtomicU64 = AtomicU64::new(0);
static PASS_CLOCK_STALE_MAX_NS: AtomicU64 = AtomicU64::new(0);

/// Lane timing wheel and pass clock totals since process start. Each field
/// is individually atomic; the snapshot is not a transaction.
///
/// The lane-clock counters of [`InlineExecutorCounters`] keep their meaning
/// with the wheel: `lane_timer_arms` counts filings (a new or moved key),
/// `lane_timer_wakes` entries taken due, `lane_timer_fires` runtime timer
/// expiries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct LaneWheelCounters {
    /// Deadlines linked into a wheel bucket when filed.
    pub wheel_inserts: u64,
    /// Deadlines filed beyond the wheel's span (overflow heap).
    pub wheel_far_inserts: u64,
    /// Filed deadlines removed before the wheel served them (re-armed,
    /// observed due by their sleep, or dropped).
    pub wheel_cancels: u64,
    /// Far deadlines moved from the overflow heap into a bucket.
    pub wheel_cascades: u64,
    /// Entry-by-entry checks of the current bucket.
    pub wheel_slot_scans: u64,
    /// Clock reads answered from a pass reading (each a clock read saved).
    pub pass_clock_cached_reads: u64,
    /// Clock reads the pass clock made (each pass start and every refresh).
    pub pass_clock_fresh_reads: u64,
    /// Pass readings replaced by a fresh one.
    pub pass_clock_staleness_samples: u64,
    /// Sum of the ages pass readings reached when replaced, in nanoseconds.
    pub pass_clock_staleness_sum_ns: u64,
    /// The largest age a pass reading reached when replaced, in nanoseconds.
    pub pass_clock_staleness_max_ns: u64,
}

/// Snapshot of the lane timing wheel and pass clock totals.
pub fn lane_wheel_counters() -> LaneWheelCounters {
    LaneWheelCounters {
        wheel_inserts: WHEEL_INSERTS.load(Ordering::Relaxed),
        wheel_far_inserts: WHEEL_FAR_INSERTS.load(Ordering::Relaxed),
        wheel_cancels: WHEEL_CANCELS.load(Ordering::Relaxed),
        wheel_cascades: WHEEL_CASCADES.load(Ordering::Relaxed),
        wheel_slot_scans: WHEEL_SLOT_SCANS.load(Ordering::Relaxed),
        pass_clock_cached_reads: PASS_CLOCK_CACHED.load(Ordering::Relaxed),
        pass_clock_fresh_reads: PASS_CLOCK_FRESH.load(Ordering::Relaxed),
        pass_clock_staleness_samples: PASS_CLOCK_STALE_SAMPLES.load(Ordering::Relaxed),
        pass_clock_staleness_sum_ns: PASS_CLOCK_STALE_SUM_NS.load(Ordering::Relaxed),
        pass_clock_staleness_max_ns: PASS_CLOCK_STALE_MAX_NS.load(Ordering::Relaxed),
    }
}

/// Timing-wheel and pass-clock counters kept by one poller task and
/// published in batches, like [`LaneTimerCounters`].
#[derive(Default)]
pub(crate) struct LaneWheelCounterBatch {
    ops: crate::lane_clock::WheelOps,
    pass: crate::lane_clock::PassClockStats,
    passes: u64,
}

impl LaneWheelCounterBatch {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// One poller pass: its wheel operations and pass-clock statistics.
    #[inline]
    pub(crate) fn pass(
        &mut self,
        ops: crate::lane_clock::WheelOps,
        pass: crate::lane_clock::PassClockStats,
    ) {
        self.ops.inserts += ops.inserts;
        self.ops.far_inserts += ops.far_inserts;
        self.ops.cancels += ops.cancels;
        self.ops.cascades += ops.cascades;
        self.ops.slot_scans += ops.slot_scans;
        self.pass.cached += pass.cached;
        self.pass.fresh += pass.fresh;
        self.pass.stale_samples += pass.stale_samples;
        self.pass.stale_sum_ns = self.pass.stale_sum_ns.saturating_add(pass.stale_sum_ns);
        self.pass.stale_max_ns = self.pass.stale_max_ns.max(pass.stale_max_ns);
        self.passes += 1;
        if self.passes >= FLUSH_EVERY {
            self.flush();
        }
    }

    pub(crate) fn flush(&mut self) {
        self.passes = 0;
        publish(&WHEEL_INSERTS, &mut self.ops.inserts);
        publish(&WHEEL_FAR_INSERTS, &mut self.ops.far_inserts);
        publish(&WHEEL_CANCELS, &mut self.ops.cancels);
        publish(&WHEEL_CASCADES, &mut self.ops.cascades);
        publish(&WHEEL_SLOT_SCANS, &mut self.ops.slot_scans);
        publish(&PASS_CLOCK_CACHED, &mut self.pass.cached);
        publish(&PASS_CLOCK_FRESH, &mut self.pass.fresh);
        publish(&PASS_CLOCK_STALE_SAMPLES, &mut self.pass.stale_samples);
        publish(&PASS_CLOCK_STALE_SUM_NS, &mut self.pass.stale_sum_ns);
        let max = std::mem::take(&mut self.pass.stale_max_ns);
        if max > 0 {
            PASS_CLOCK_STALE_MAX_NS.fetch_max(max, Ordering::Relaxed);
        }
    }
}

impl Drop for LaneWheelCounterBatch {
    fn drop(&mut self) {
        self.flush();
    }
}

// ---- End lane timing wheel and pass clock ----

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

/// Count one receive loop that owns its inbound SRTP context.
pub(crate) fn record_owner_srtp_loop() {
    OWNER_SRTP_LOOPS.fetch_add(1, Ordering::Relaxed);
}

/// Count one adoption or return of the inbound SRTP context by its owner.
/// Rekey rate.
pub(crate) fn record_owner_srtp_handoff() {
    OWNER_SRTP_HANDOFFS.fetch_add(1, Ordering::Relaxed);
}

/// Count one inbound RTCP compound processed on the socket owner. RTCP rate
/// (a few seconds apart per call); batched with the lane pass when the
/// poller defers (see [`add_lane_deferred`]).
pub(crate) fn record_owner_rtcp_inbound() {
    add_lane_deferred(&OWNER_RTCP_INBOUND, 1);
    add_lane_deferred(&WAKEUPS_AVOIDED, 1);
}

/// Count one synchronous periodic report send (`synchronous`), or one the
/// socket deferred to the awaited send; batched with the lane pass when the
/// poller defers.
pub(crate) fn record_rtcp_send(synchronous: bool) {
    add_lane_deferred(
        if synchronous {
            &RTCP_SYNC_SENDS
        } else {
            &RTCP_SYNC_DEFERRED
        },
        1,
    );
}

// ---- Counter and PCM pool cleanup: lane-deferred counters ----

/// Lane passes a deferring poller runs between two publications of its
/// thread's pending [`add_lane_deferred`] values. It also publishes them on
/// every idle pass and when it stops.
pub(crate) const LANE_DEFERRED_FLUSH_PASSES: u32 = 64;

/// Distinct counters one thread holds pending; a value for a further counter
/// is added at once.
const LANE_DEFERRED_SLOTS: usize = 8;

/// One thread's pending lane-deferred values.
struct LaneDeferredTable {
    /// Inside a deferring scope (a lane poller pass).
    deferring: bool,
    /// Counters in first-use order, each with its pending value.
    slots: [(Option<&'static AtomicU64>, u64); LANE_DEFERRED_SLOTS],
}

impl LaneDeferredTable {
    const fn new() -> Self {
        Self {
            deferring: false,
            slots: [(None, 0); LANE_DEFERRED_SLOTS],
        }
    }

    /// Hold `value` for `total`; `false` when every slot holds another
    /// counter.
    #[inline]
    fn queue(&mut self, total: &'static AtomicU64, value: u64) -> bool {
        for slot in &mut self.slots {
            match slot.0 {
                Some(held) if std::ptr::eq(held, total) => {
                    slot.1 += value;
                    return true;
                }
                Some(_) => {}
                None => {
                    *slot = (Some(total), value);
                    return true;
                }
            }
        }
        false
    }

    fn pending(&self, total: &AtomicU64) -> u64 {
        self.slots
            .iter()
            .find(|(held, _)| held.is_some_and(|held| std::ptr::eq(held, total)))
            .map_or(0, |&(_, pending)| pending)
    }

    fn flush(&mut self) {
        for (total, pending) in &mut self.slots {
            let Some(total) = total else {
                break; // slots fill in order
            };
            if *pending > 0 {
                publish_lane_deferred(total, std::mem::take(pending));
            }
        }
    }
}

impl Drop for LaneDeferredTable {
    fn drop(&mut self) {
        // Thread exit: nothing held here is lost.
        self.flush();
    }
}

thread_local! {
    static LANE_DEFERRED: std::cell::RefCell<LaneDeferredTable> =
        const { std::cell::RefCell::new(LaneDeferredTable::new()) };
}

#[cfg(test)]
thread_local! {
    /// Atomic adds this thread made for lane-deferred counters (tests).
    static LANE_DEFERRED_PUBLISHES: Cell<u64> = const { Cell::new(0) };
}

#[inline]
fn publish_lane_deferred(total: &AtomicU64, value: u64) {
    total.fetch_add(value, Ordering::Relaxed);
    #[cfg(test)]
    let _ = LANE_DEFERRED_PUBLISHES.try_with(|count| count.set(count.get() + 1));
}

/// Run `apply` on this thread's table; `None` during thread teardown or a
/// re-entrant call.
#[inline]
fn with_lane_deferred<R>(apply: impl FnOnce(&mut LaneDeferredTable) -> R) -> Option<R> {
    LANE_DEFERRED
        .try_with(|table| {
            table
                .try_borrow_mut()
                .ok()
                .map(|mut table| apply(&mut table))
        })
        .ok()
        .flatten()
}

/// Add `value` to `total`, a process counter of the media path that is
/// updated per event (PCM pool misses, owner RTCP) rather than kept by an
/// owner and published in batches.
///
/// During a lane poller pass with
/// [`MediaExecutorConfig::counter_pool_cleanup`] active on a current-thread
/// runtime (whose poller never changes thread), the value is held in a small
/// per-thread table and the poller publishes the table every 64 passes, on
/// every idle pass and when it stops; a thread's pending values are also
/// published when it exits. Everywhere else the value is added at once.
/// Either way every value reaches `total` exactly once, so readers see the
/// same totals, at most a batch of lane passes later.
#[inline]
pub fn add_lane_deferred(total: &'static AtomicU64, value: u64) {
    let held = with_lane_deferred(|table| table.deferring && table.queue(total, value));
    if held != Some(true) {
        publish_lane_deferred(total, value);
    }
}

/// While alive, [`add_lane_deferred`] on this thread holds values instead of
/// adding them. Dropping it restores the previous state and publishes
/// nothing.
pub(crate) struct LaneDeferredScope {
    previous: bool,
}

impl LaneDeferredScope {
    fn enter() -> Self {
        let previous = with_lane_deferred(|table| std::mem::replace(&mut table.deferring, true))
            .unwrap_or(false);
        Self { previous }
    }
}

impl Drop for LaneDeferredScope {
    fn drop(&mut self) {
        let previous = self.previous;
        with_lane_deferred(|table| table.deferring = previous);
    }
}

/// Publish everything this thread holds pending.
fn flush_lane_deferred() {
    with_lane_deferred(LaneDeferredTable::flush);
}

/// A lane poller's schedule for publishing the values its passes hold.
pub(crate) struct LaneDeferredBatch {
    passes: u32,
}

impl LaneDeferredBatch {
    /// The schedule for a poller starting on this thread, or `None` when it
    /// adds at once: [`MediaExecutorConfig::counter_pool_cleanup`] is not
    /// active, or the runtime is multi-threaded (its poller may move between
    /// workers, leaving values behind on a thread it no longer runs on).
    pub(crate) fn for_this_poller() -> Option<Self> {
        let current_thread = tokio::runtime::Handle::try_current().is_ok_and(|handle| {
            handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::CurrentThread
        });
        (current_thread && counter_pool_cleanup_enabled()).then_some(Self { passes: 0 })
    }

    /// Hold this thread's lane-deferred values until the scope drops (one
    /// poller pass).
    pub(crate) fn enter_pass(&self) -> LaneDeferredScope {
        LaneDeferredScope::enter()
    }

    /// A pass ended. `idle`: it polled no hosted future, or the poller hosts
    /// none (the lane may now sleep for long).
    #[inline]
    pub(crate) fn pass_end(&mut self, idle: bool) {
        self.passes += 1;
        if idle || self.passes >= LANE_DEFERRED_FLUSH_PASSES {
            self.passes = 0;
            flush_lane_deferred();
        }
    }
}

impl Drop for LaneDeferredBatch {
    fn drop(&mut self) {
        flush_lane_deferred();
    }
}

/// Restores this thread's previous state when dropped, after publishing what
/// it holds pending.
#[doc(hidden)]
pub struct LaneDeferredGuard {
    _scope: LaneDeferredScope,
}

impl Drop for LaneDeferredGuard {
    fn drop(&mut self) {
        flush_lane_deferred();
    }
}

/// Test seam: hold [`add_lane_deferred`] values on this thread, as a
/// deferring lane poller pass does, until the guard drops.
#[doc(hidden)]
pub fn defer_lane_counters_on_this_thread() -> LaneDeferredGuard {
    LaneDeferredGuard {
        _scope: LaneDeferredScope::enter(),
    }
}

/// Test seam: the value this thread holds for `total` and has not yet
/// added to it.
#[doc(hidden)]
pub fn pending_lane_deferred_on_this_thread(total: &AtomicU64) -> u64 {
    with_lane_deferred(|table| table.pending(total)).unwrap_or(0)
}

// ---- End counter and PCM pool cleanup: lane-deferred counters ----

/// Count one periodic report task hosted on a lane poller.
pub(crate) fn record_rtcp_lane_task() {
    RTCP_LANE_TASKS.fetch_add(1, Ordering::Relaxed);
}

/// Count one session whose socket owner publishes its receive statistics.
pub(crate) fn record_owner_receive_stats() {
    OWNER_RECEIVE_STATS.fetch_add(1, Ordering::Relaxed);
}

/// Lock-free SRTP datagrams of one owning receive loop, published in
/// batches.
#[derive(Default)]
pub(crate) struct SrtpOwnerCounters {
    pending: u64,
}

impl SrtpOwnerCounters {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub(crate) fn lock_avoided(&mut self) {
        self.pending += 1;
        if self.pending >= FLUSH_EVERY {
            self.flush();
        }
    }

    pub(crate) fn flush(&mut self) {
        publish(&SRTP_LOCKS_AVOIDED, &mut self.pending);
    }
}

impl Drop for SrtpOwnerCounters {
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
        assert!(!v1.inline_decode_active());
        assert!(!v1.pcm_pool_active());
        assert!(!v1.owner_srtp_active());
        assert!(!v1.owner_rtcp_active());
        assert!(!v1.owner_rtcp_on_lane_active());
        assert!(!v1.owner_receive_stats_active());

        let v2 = MediaExecutorConfig::v2();
        assert_eq!(v2.mode, MediaExecutorMode::V2);
        assert!(v2.lane_poller_active());
        assert!(v2.lane_timer_active());
        assert!(v2.lane_tx_active());
        assert!(v2.tx_batch_active());
        assert!(v2.fast_ssrc_hash_active());
        assert!(v2.inline_decode_active());
        assert!(v2.pcm_pool_active());
        assert!(v2.owner_srtp_active());
        assert!(v2.owner_rtcp_active());
        assert!(v2.owner_rtcp_on_lane_active());
        assert!(v2.owner_receive_stats_active());
        assert!(v2.validate().is_ok());
    }

    // ---- Lane tick ----

    #[test]
    fn the_lane_tick_is_off_in_v1_and_a_2ms_grid_tick_in_v2() {
        let v1 = MediaExecutorConfig::v1();
        assert_eq!(v1.lane_tick, None);
        assert_eq!(v1.lane_tick_active(), None);
        assert!(!v1.lane_tick_grid_timers_active());

        let v2 = MediaExecutorConfig::v2();
        let tick = v2.lane_tick.expect("v2 ticks");
        assert_eq!(tick.quantum, Duration::from_millis(2));
        assert!(tick.grid_timers);
        assert_eq!(v2.lane_tick_active(), Some(tick));
        assert!(v2.lane_tick_grid_timers_active());

        assert_eq!(
            v2.clone()
                .with_mode(MediaExecutorMode::V1)
                .lane_tick_active(),
            None,
            "never under v1"
        );
        let no_poller = v2.clone().with_lane_poller(false);
        assert!(
            no_poller.validate().is_ok(),
            "a tick without the poller reads as off"
        );
        assert_eq!(no_poller.lane_tick_active(), None);
        assert!(!no_poller.lane_tick_grid_timers_active());
        let no_clock = v2.clone().with_lane_timer(false);
        assert_eq!(
            no_clock.lane_tick_active(),
            Some(tick),
            "the tick needs no lane clock"
        );
        assert!(!no_clock.lane_tick_grid_timers_active(), "grid timers do");
        let no_grid = v2
            .clone()
            .with_lane_tick(Some(LaneTick::new(Duration::from_millis(5))));
        assert!(no_grid.validate().is_ok());
        assert!(!no_grid.lane_tick_grid_timers_active());
        assert_eq!(v2.with_lane_tick(None).lane_tick_active(), None);
    }

    #[test]
    fn a_lane_tick_quantum_must_divide_the_media_clock() {
        for ms in crate::lane_tick::LANE_TICK_QUANTA_MS {
            let tick = LaneTick::new(Duration::from_millis(ms)).with_grid_timers(true);
            assert!(MediaExecutorConfig::v2()
                .with_lane_tick(Some(tick))
                .validate()
                .is_ok());
        }
        for quantum in [
            Duration::ZERO,
            Duration::from_micros(2_500),
            Duration::from_millis(3),
            Duration::from_millis(10),
        ] {
            let config = MediaExecutorConfig::v1().with_lane_tick(Some(LaneTick::new(quantum)));
            assert_eq!(
                config.validate(),
                Err(MediaExecutorConfigError::InvalidLaneTickQuantum { quantum }),
                "an invalid value is refused even where it would read as off"
            );
            let slot = OnceLock::new();
            assert!(install_into(&slot, config).is_err());
            assert!(slot.get().is_none(), "nothing was installed");
        }
        let lane_tick = |tick: Option<LaneTick>| {
            let _guard = override_media_executor_config_on_this_thread(
                MediaExecutorConfig::v2().with_lane_tick(tick),
            );
            (active_lane_tick(), lane_tick_grid_timers_enabled())
        };
        let tick = LaneTick::new(Duration::from_millis(4));
        assert_eq!(lane_tick(Some(tick)), (Some(tick), false));
        assert_eq!(
            lane_tick(Some(tick.with_grid_timers(true))),
            (Some(tick.with_grid_timers(true)), true)
        );
        assert_eq!(lane_tick(None), (None, false));
    }

    // ---- End lane tick ----

    // ---- Lane timing wheel and pass clock ----

    #[test]
    fn the_timing_wheel_and_pass_clock_are_off_in_v1_and_on_in_v2() {
        let v1 = MediaExecutorConfig::v1();
        assert!(!v1.lane_timer_wheel && !v1.lane_pass_clock);
        assert!(!v1.lane_timer_wheel_active());
        assert!(!v1.lane_pass_clock_active());
        let v2 = MediaExecutorConfig::v2();
        assert!(v2.lane_timer_wheel && v2.lane_pass_clock);
        assert!(v2.lane_timer_wheel_active());
        assert!(v2.lane_pass_clock_active());
        assert!(v2.validate().is_ok());

        // Never under v1, whatever the fields say.
        let v1_with_fields = v2.clone().with_mode(MediaExecutorMode::V1);
        assert!(!v1_with_fields.lane_timer_wheel_active());
        assert!(!v1_with_fields.lane_pass_clock_active());

        // The wheel needs the lane clock; the pass clock only the poller.
        // A missing prerequisite reads as off and is not an error.
        let no_clock = MediaExecutorConfig::v2().with_lane_timer(false);
        assert!(no_clock.validate().is_ok());
        assert!(!no_clock.lane_timer_wheel_active());
        assert!(no_clock.lane_pass_clock_active());
        let no_poller = MediaExecutorConfig::v2().with_lane_poller(false);
        assert!(no_poller.validate().is_ok());
        assert!(!no_poller.lane_timer_wheel_active());
        assert!(!no_poller.lane_pass_clock_active());

        // Each part alone.
        let wheel_only = MediaExecutorConfig::v2().with_lane_pass_clock(false);
        assert!(wheel_only.lane_timer_wheel_active() && !wheel_only.lane_pass_clock_active());
        let pass_only = MediaExecutorConfig::v2().with_lane_timer_wheel(false);
        assert!(!pass_only.lane_timer_wheel_active() && pass_only.lane_pass_clock_active());

        // The process accessors follow the configuration in effect.
        let accessors = |config: MediaExecutorConfig| {
            let _guard = override_media_executor_config_on_this_thread(config);
            (lane_timer_wheel_enabled(), lane_pass_clock_enabled())
        };
        assert_eq!(accessors(MediaExecutorConfig::v1()), (false, false));
        assert_eq!(accessors(MediaExecutorConfig::v2()), (true, true));
        assert_eq!(accessors(no_clock), (false, true));
        assert_eq!(accessors(wheel_only), (true, false));
    }

    #[test]
    fn lane_wheel_counters_flush_in_batches_and_on_drop() {
        let before = lane_wheel_counters();
        let mut batch = LaneWheelCounterBatch::new();
        let ops = crate::lane_clock::WheelOps {
            inserts: 3,
            far_inserts: 1,
            cancels: 2,
            cascades: 1,
            slot_scans: 4,
        };
        let pass = crate::lane_clock::PassClockStats {
            cached: 30,
            fresh: 2,
            stale_samples: 1,
            stale_sum_ns: 40_000,
            stale_max_ns: 40_000,
        };
        batch.pass(ops, pass);
        drop(batch);
        let after = lane_wheel_counters();
        assert!(after.wheel_inserts >= before.wheel_inserts + 3);
        assert!(after.wheel_far_inserts > before.wheel_far_inserts);
        assert!(after.wheel_cancels >= before.wheel_cancels + 2);
        assert!(after.wheel_cascades > before.wheel_cascades);
        assert!(after.wheel_slot_scans >= before.wheel_slot_scans + 4);
        assert!(after.pass_clock_cached_reads >= before.pass_clock_cached_reads + 30);
        assert!(after.pass_clock_fresh_reads >= before.pass_clock_fresh_reads + 2);
        assert!(after.pass_clock_staleness_samples > before.pass_clock_staleness_samples);
        assert!(after.pass_clock_staleness_sum_ns >= before.pass_clock_staleness_sum_ns + 40_000);
        assert!(after.pass_clock_staleness_max_ns >= 40_000);

        // A full batch publishes without waiting for the drop.
        let before = lane_wheel_counters();
        let mut batch = LaneWheelCounterBatch::new();
        for _ in 0..FLUSH_EVERY {
            batch.pass(
                crate::lane_clock::WheelOps {
                    inserts: 1,
                    ..Default::default()
                },
                crate::lane_clock::PassClockStats::default(),
            );
        }
        assert!(lane_wheel_counters().wheel_inserts >= before.wheel_inserts + FLUSH_EVERY);
        assert_eq!(batch.passes, 0, "flushed when full");
    }

    // ---- End lane timing wheel and pass clock ----

    #[test]
    fn switches_never_apply_under_v1_and_dependents_follow_the_lane_poller() {
        let v1_with_switches = MediaExecutorConfig::v2().with_mode(MediaExecutorMode::V1);
        assert!(!v1_with_switches.lane_poller_active(), "never under v1");
        assert!(!v1_with_switches.lane_timer_active(), "never under v1");
        assert!(!v1_with_switches.tx_batch_active(), "never under v1");
        assert!(!v1_with_switches.fast_ssrc_hash_active(), "never under v1");
        assert!(!v1_with_switches.inline_decode_active(), "never under v1");
        assert!(!v1_with_switches.pcm_pool_active(), "never under v1");
        assert!(!v1_with_switches.owner_srtp_active(), "never under v1");
        assert!(!v1_with_switches.owner_rtcp_active(), "never under v1");
        assert!(
            !v1_with_switches.owner_receive_stats_active(),
            "never under v1"
        );

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
        assert!(
            no_poller.inline_decode_active() && no_poller.pcm_pool_active(),
            "inline decode and the PCM pool do not need the poller"
        );

        let no_inline = MediaExecutorConfig::v2()
            .with_inline_decode(false)
            .with_pcm_pool(false);
        assert!(no_inline.validate().is_ok());
        assert!(!no_inline.inline_decode_active());
        assert!(!no_inline.pcm_pool_active());
        assert!(no_inline.lane_poller_active() && no_inline.tx_batch_active());
        assert!(MediaExecutorConfig::v2()
            .with_inline_decode(false)
            .pcm_pool_active());
        assert!(
            no_poller.owner_srtp_active(),
            "a receive task owns its context as well as a hosted loop"
        );
        assert!(no_poller.owner_rtcp_active());
        assert!(
            !no_poller.owner_rtcp_on_lane_active(),
            "without the poller the report task stays a task"
        );

        let one_off = MediaExecutorConfig::v2()
            .with_lane_timer(false)
            .with_lane_tx(false)
            .with_tx_batch(false)
            .with_fast_ssrc_hash(false)
            .with_owner_srtp(false)
            .with_owner_rtcp(false)
            .with_owner_receive_stats(false);
        assert!(one_off.lane_poller_active());
        assert!(!one_off.lane_timer_active());
        assert!(!one_off.lane_tx_active());
        assert!(!one_off.tx_batch_active());
        assert!(!one_off.fast_ssrc_hash_active());
        assert!(!one_off.owner_srtp_active());
        assert!(!one_off.owner_rtcp_active());
        assert!(!one_off.owner_rtcp_on_lane_active());
        assert!(!one_off.owner_receive_stats_active());
    }

    #[test]
    fn owner_srtp_counters_reach_the_totals() {
        let before = inline_executor_counters();
        let mut counters = SrtpOwnerCounters::new();
        for _ in 0..(FLUSH_EVERY + 2) {
            counters.lock_avoided();
        }
        let flushed = inline_executor_counters();
        assert!(flushed.srtp_locks_avoided >= before.srtp_locks_avoided + FLUSH_EVERY);
        drop(counters);
        record_owner_srtp_loop();
        record_owner_srtp_handoff();
        let after = inline_executor_counters();
        assert!(after.srtp_locks_avoided >= before.srtp_locks_avoided + FLUSH_EVERY + 2);
        assert!(after.owner_srtp_loops > before.owner_srtp_loops);
        assert!(after.owner_srtp_handoffs > before.owner_srtp_handoffs);
    }

    #[test]
    fn owner_rtcp_counters_reach_the_totals() {
        let before = inline_executor_counters();
        record_owner_rtcp_inbound();
        record_rtcp_send(true);
        record_rtcp_send(false);
        record_rtcp_lane_task();
        let after = inline_executor_counters();
        assert!(after.owner_rtcp_inbound > before.owner_rtcp_inbound);
        assert!(after.wakeups_avoided > before.wakeups_avoided);
        assert!(after.rtcp_sync_sends > before.rtcp_sync_sends);
        assert!(after.rtcp_sync_deferred > before.rtcp_sync_deferred);
        assert!(after.rtcp_lane_tasks > before.rtcp_lane_tasks);
    }

    #[test]
    fn owner_receive_stats_sessions_are_counted() {
        let before = inline_executor_counters();
        record_owner_receive_stats();
        assert!(inline_executor_counters().owner_receive_stats > before.owner_receive_stats);
    }

    // ---- Hot call state ----

    #[test]
    fn hot_state_is_off_in_v1_on_in_v2_and_never_applies_under_v1() {
        assert!(!MediaExecutorConfig::v1().hot_state);
        assert!(!MediaExecutorConfig::v1().hot_state_active());
        assert!(MediaExecutorConfig::v2().hot_state);
        assert!(MediaExecutorConfig::v2().hot_state_active());
        assert!(MediaExecutorConfig::v2().validate().is_ok());
        assert!(
            !MediaExecutorConfig::v1()
                .with_hot_state(true)
                .hot_state_active(),
            "never under v1"
        );
        let off = MediaExecutorConfig::v2().with_hot_state(false);
        assert!(!off.hot_state_active());
        assert!(off.validate().is_ok());
        // No prerequisite: it applies without the lane poller (the receive
        // loop's entry then stays local to the loop).
        assert!(MediaExecutorConfig::v2()
            .with_lane_poller(false)
            .hot_state_active());
        {
            let _v2 = override_media_executor_config_on_this_thread(MediaExecutorConfig::v2());
            assert!(hot_state_enabled());
            let _off = override_media_executor_config_on_this_thread(off);
            assert!(!hot_state_enabled());
        }
    }

    // ---- End hot call state ----

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
        assert!(inline_decode_enabled() && pcm_pool_enabled());
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
            assert!(!inline_decode_enabled() && !pcm_pool_enabled());
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

    // ---- Counter and PCM pool cleanup ----

    #[test]
    fn counter_pool_cleanup_is_part_of_v2_and_off_under_v1() {
        assert!(!MediaExecutorConfig::v1().counter_pool_cleanup);
        assert!(!MediaExecutorConfig::v1().counter_pool_cleanup_active());
        assert!(MediaExecutorConfig::v2().counter_pool_cleanup_active());
        assert!(
            !MediaExecutorConfig::v1()
                .with_counter_pool_cleanup(true)
                .counter_pool_cleanup_active(),
            "never under v1"
        );
        let off = MediaExecutorConfig::v2().with_counter_pool_cleanup(false);
        assert!(!off.counter_pool_cleanup_active());
        assert!(off.pcm_pool_active(), "independent of the pool switch");
        off.validate().unwrap();
        MediaExecutorConfig::v2()
            .with_lane_poller(false)
            .validate()
            .unwrap();
        let _v2 = override_media_executor_config_on_this_thread(MediaExecutorConfig::v2());
        assert!(counter_pool_cleanup_enabled());
        let _off = override_media_executor_config_on_this_thread(off);
        assert!(!counter_pool_cleanup_enabled());
    }

    /// Only a poller on a current-thread runtime with the switch defers: a
    /// multi-threaded runtime may move the poller between workers.
    #[test]
    fn only_a_current_thread_poller_with_the_switch_defers() {
        std::thread::spawn(|| {
            let current = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            let multi = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .build()
                .unwrap();
            let defers = |runtime: &tokio::runtime::Runtime| {
                runtime.block_on(async { LaneDeferredBatch::for_this_poller().is_some() })
            };
            let _v2 = override_media_executor_config_on_this_thread(MediaExecutorConfig::v2());
            assert!(defers(&current));
            assert!(!defers(&multi));
            assert!(
                LaneDeferredBatch::for_this_poller().is_none(),
                "outside a runtime"
            );
            let _off = override_media_executor_config_on_this_thread(
                MediaExecutorConfig::v2().with_counter_pool_cleanup(false),
            );
            assert!(!defers(&current));
            let _v1 = override_media_executor_config_on_this_thread(MediaExecutorConfig::v1());
            assert!(!defers(&current));
        })
        .join()
        .unwrap();
    }

    fn lane_deferred_publishes() -> u64 {
        LANE_DEFERRED_PUBLISHES.with(Cell::get)
    }

    /// The same events, counted per event and through a deferring lane batch:
    /// identical totals, one atomic add per counter per batch instead of one
    /// per event.
    #[test]
    fn lane_deferred_counts_reach_the_same_totals_with_fewer_atomic_adds() {
        static PER_EVENT_A: AtomicU64 = AtomicU64::new(0);
        static PER_EVENT_B: AtomicU64 = AtomicU64::new(0);
        static BATCHED_A: AtomicU64 = AtomicU64::new(0);
        static BATCHED_B: AtomicU64 = AtomicU64::new(0);
        const PASSES: u32 = 200;
        std::thread::spawn(|| {
            let start = lane_deferred_publishes();
            for pass in 0..PASSES {
                add_lane_deferred(&PER_EVENT_A, 1);
                add_lane_deferred(&PER_EVENT_B, u64::from(pass % 3));
            }
            let per_event = lane_deferred_publishes() - start;

            let mut batch = LaneDeferredBatch { passes: 0 };
            let start = lane_deferred_publishes();
            for pass in 0..PASSES {
                let _pass = batch.enter_pass();
                add_lane_deferred(&BATCHED_A, 1);
                add_lane_deferred(&BATCHED_B, u64::from(pass % 3));
                if pass == 10 {
                    assert_eq!(pending_lane_deferred_on_this_thread(&BATCHED_A), 11);
                    assert_eq!(BATCHED_A.load(Ordering::Relaxed), 0, "held, not added");
                }
                batch.pass_end(false);
            }
            drop(batch);
            let batched = lane_deferred_publishes() - start;

            assert_eq!(
                BATCHED_A.load(Ordering::Relaxed),
                PER_EVENT_A.load(Ordering::Relaxed)
            );
            assert_eq!(
                BATCHED_B.load(Ordering::Relaxed),
                PER_EVENT_B.load(Ordering::Relaxed)
            );
            assert_eq!(PER_EVENT_A.load(Ordering::Relaxed), u64::from(PASSES));
            assert_eq!(per_event, 2 * u64::from(PASSES));
            // Flushes after passes 64, 128 and 192, and when the batch drops.
            assert_eq!(batched, 2 * 4);
            assert_eq!(pending_lane_deferred_on_this_thread(&BATCHED_A), 0);
        })
        .join()
        .unwrap();
    }

    /// An idle pass publishes at once; outside a pass values are added at
    /// once even while others are held; scopes nest; a counter beyond the
    /// table's slots is added at once; a thread's held values survive its
    /// exit.
    #[test]
    fn lane_deferred_values_are_published_exactly_once_on_every_path() {
        static IDLE: AtomicU64 = AtomicU64::new(0);
        static OUTSIDE: AtomicU64 = AtomicU64::new(0);
        static EXITED: AtomicU64 = AtomicU64::new(0);
        static MANY: [AtomicU64; LANE_DEFERRED_SLOTS + 2] =
            [const { AtomicU64::new(0) }; LANE_DEFERRED_SLOTS + 2];
        std::thread::spawn(|| {
            let mut batch = LaneDeferredBatch { passes: 0 };
            {
                let _pass = batch.enter_pass();
                add_lane_deferred(&IDLE, 3);
                assert_eq!(IDLE.load(Ordering::Relaxed), 0);
                batch.pass_end(true);
            }
            assert_eq!(IDLE.load(Ordering::Relaxed), 3, "an idle pass publishes");

            {
                let _pass = batch.enter_pass();
                add_lane_deferred(&OUTSIDE, 1);
                {
                    let _nested = batch.enter_pass();
                    add_lane_deferred(&OUTSIDE, 1);
                }
                assert!(
                    with_lane_deferred(|table| table.deferring).unwrap(),
                    "a nested scope restores the outer one"
                );
                batch.pass_end(false);
            }
            assert!(!with_lane_deferred(|table| table.deferring).unwrap());
            add_lane_deferred(&OUTSIDE, 5);
            assert_eq!(
                OUTSIDE.load(Ordering::Relaxed),
                5,
                "outside a pass: at once"
            );
            assert_eq!(pending_lane_deferred_on_this_thread(&OUTSIDE), 2);
            drop(batch);
            assert_eq!(OUTSIDE.load(Ordering::Relaxed), 7);

            // Every slot is taken by now (IDLE, OUTSIDE, then MANY): the
            // counters that do not fit are added at once.
            let guard = defer_lane_counters_on_this_thread();
            for counter in &MANY {
                add_lane_deferred(counter, 1);
            }
            let held = MANY
                .iter()
                .filter(|counter| counter.load(Ordering::Relaxed) == 0)
                .count();
            assert_eq!(held, LANE_DEFERRED_SLOTS - 2);
            drop(guard);
            assert!(MANY
                .iter()
                .all(|counter| counter.load(Ordering::Relaxed) == 1));

            // Held when the thread exits.
            let _pass = LaneDeferredScope::enter();
            add_lane_deferred(&EXITED, 9);
            std::mem::forget(_pass);
        })
        .join()
        .unwrap();
        assert_eq!(
            EXITED.load(Ordering::Relaxed),
            9,
            "thread exit published it"
        );
    }

    /// The owner RTCP counters count the same through a deferring pass.
    #[test]
    fn owner_rtcp_counts_are_held_for_the_lane_batch_and_reach_the_totals() {
        std::thread::spawn(|| {
            let before = inline_executor_counters();
            let guard = defer_lane_counters_on_this_thread();
            record_owner_rtcp_inbound();
            record_rtcp_send(true);
            record_rtcp_send(true);
            record_rtcp_send(false);
            assert_eq!(pending_lane_deferred_on_this_thread(&OWNER_RTCP_INBOUND), 1);
            assert_eq!(pending_lane_deferred_on_this_thread(&WAKEUPS_AVOIDED), 1);
            assert_eq!(pending_lane_deferred_on_this_thread(&RTCP_SYNC_SENDS), 2);
            assert_eq!(pending_lane_deferred_on_this_thread(&RTCP_SYNC_DEFERRED), 1);
            drop(guard);
            assert_eq!(pending_lane_deferred_on_this_thread(&RTCP_SYNC_SENDS), 0);
            let after = inline_executor_counters();
            assert!(after.owner_rtcp_inbound > before.owner_rtcp_inbound);
            assert!(after.wakeups_avoided > before.wakeups_avoided);
            assert!(after.rtcp_sync_sends >= before.rtcp_sync_sends + 2);
            assert!(after.rtcp_sync_deferred > before.rtcp_sync_deferred);
        })
        .join()
        .unwrap();
    }

    // ---- End counter and PCM pool cleanup ----
}
