//! Runtime isolation for RTP/media-plane tasks.
//!
//! A dense media workload can have thousands of 20 ms timers and socket
//! continuations ready at once. Running those continuations on the caller's
//! signaling executor allows valid RTP load to delay SIP transaction timers
//! and response handling. Media remains lifecycle-owned by its `JoinHandle`,
//! but its polling budget is supplied by this dedicated executor.
//!
//! # Lanes
//!
//! The media executor is a pool of one or more *lanes*. Each lane is an
//! independent Tokio runtime with its own scheduler, timer driver and I/O
//! reactor (epoll instance on Linux). By default there is a single lane, which
//! is the layout every earlier release used. A host with many cores can split
//! the media worker budget across several lanes with
//! [`install_media_runtime_config`], so that thousands of sockets and timers
//! no longer serialize on one reactor and one timer driver.
//!
//! Lanes come in two kinds, selected by [`MediaLanes`]:
//!
//! - [`MediaLanes::Shared`] lanes are multi-threaded runtimes that split one
//!   worker-thread budget.
//! - [`MediaLanes::Dedicated`] lanes are single-threaded (current-thread)
//!   runtimes, each driven by its own OS thread, which can be pinned to one
//!   CPU. A task placed on a dedicated lane is polled only by that thread, so
//!   its timers, sockets and wakeups never migrate between cores.
//!
//! Each UDP transport selects one lane when it is created. Its sockets are
//! registered on that lane's reactor and its receive tasks run on that lane,
//! so readiness for a media socket never wakes the signaling runtime that
//! created it. Socket placement uses its own round-robin counter, independent
//! of other task creation, so a fixed number of task spawns per call cannot
//! alias every socket onto one lane.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::future::Future;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::OnceLock;

use tokio::runtime::{Builder, Handle, Runtime};
use tokio::task::JoinHandle;

/// Largest number of shared lanes [`MediaLanes::Shared`] accepts.
pub const MAX_SHARED_MEDIA_LANES: usize = 8;

/// Largest explicit media worker-thread budget [`MediaLanes::Shared`] accepts.
pub const MAX_MEDIA_WORKER_THREADS: usize = 64;

/// Largest number of lanes [`MediaLanes::Dedicated`] accepts.
pub const MAX_DEDICATED_MEDIA_LANES: usize = 1024;

/// How the media executor divides its worker threads into lanes.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MediaLanes {
    /// `lanes` multi-threaded runtimes that split one worker-thread budget.
    ///
    /// `worker_threads` is a total across all lanes, so adding lanes does not
    /// multiply the budget: 16 workers in 4 lanes is 4 workers per lane, and a
    /// remainder goes to the first lanes. `None` keeps the built-in policy
    /// (half the available parallelism, from 2 to 8 workers), raised when
    /// needed so that every lane has at least one worker.
    Shared {
        /// Total worker threads across all lanes; `None` for the built-in
        /// policy. Must be from 1 to [`MAX_MEDIA_WORKER_THREADS`] and at least
        /// `lanes` when set.
        worker_threads: Option<usize>,
        /// Number of independent runtimes, from 1 to
        /// [`MAX_SHARED_MEDIA_LANES`].
        lanes: usize,
    },
    /// One current-thread runtime per entry, each driven by its own thread.
    ///
    /// Lane `i` is pinned to CPU `cpus[i]` when that entry is `Some` (Linux
    /// only); `None` leaves the lane's thread to the scheduler. A lane whose
    /// CPU cannot be applied (outside the process's CPU set, or a platform
    /// without thread affinity) logs a warning and floats instead of failing;
    /// [`MediaLaneSnapshot::pinned_cpu`] reports what was applied. A lane is
    /// a single thread, so it suits one lane per core; a task that blocks
    /// stalls every other task on its lane.
    Dedicated {
        /// One entry per lane, from 1 to [`MAX_DEDICATED_MEDIA_LANES`]
        /// entries; pinned CPUs must be distinct.
        cpus: Vec<Option<usize>>,
    },
}

impl MediaLanes {
    /// `lanes` dedicated lanes whose threads are not pinned.
    pub fn dedicated(lanes: usize) -> Self {
        Self::Dedicated {
            cpus: vec![None; lanes],
        }
    }

    /// One dedicated lane per CPU, each pinned to its CPU.
    pub fn pinned(cpus: impl IntoIterator<Item = usize>) -> Self {
        Self::Dedicated {
            cpus: cpus.into_iter().map(Some).collect(),
        }
    }
}

impl Default for MediaLanes {
    /// One shared lane sized by the built-in worker policy.
    fn default() -> Self {
        Self::Shared {
            worker_threads: None,
            lanes: 1,
        }
    }
}

/// Layout of the process-wide media executor.
///
/// The default is one lane sized by the built-in worker policy, which is the
/// layout used when no configuration is installed.
///
/// ```
/// use rvoip_rtp_core::task_runtime::{MediaLanes, MediaRuntimeConfig};
///
/// // 16 media workers in four independent lanes of four.
/// let config = MediaRuntimeConfig::new(MediaLanes::Shared {
///     worker_threads: Some(16),
///     lanes: 4,
/// });
/// config.validate().unwrap();
///
/// // One single-threaded lane on each of CPUs 2 to 5.
/// let config = MediaRuntimeConfig::new(MediaLanes::pinned(2..6));
/// config.validate().unwrap();
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct MediaRuntimeConfig {
    /// How worker threads are divided into lanes.
    pub lanes: MediaLanes,
}

impl MediaRuntimeConfig {
    /// A configuration with the given lane layout.
    pub fn new(lanes: MediaLanes) -> Self {
        Self { lanes }
    }

    /// Check the layout without building anything.
    pub fn validate(&self) -> Result<(), MediaRuntimeConfigError> {
        match &self.lanes {
            &MediaLanes::Shared {
                worker_threads,
                lanes,
            } => {
                if !(1..=MAX_SHARED_MEDIA_LANES).contains(&lanes) {
                    return Err(MediaRuntimeConfigError::InvalidLaneCount { lanes });
                }
                if let Some(worker_threads) = worker_threads {
                    if !(1..=MAX_MEDIA_WORKER_THREADS).contains(&worker_threads) {
                        return Err(MediaRuntimeConfigError::InvalidWorkerThreads {
                            worker_threads,
                        });
                    }
                    if lanes > worker_threads {
                        return Err(MediaRuntimeConfigError::LanesExceedWorkerThreads {
                            lanes,
                            worker_threads,
                        });
                    }
                }
                Ok(())
            }
            MediaLanes::Dedicated { cpus } => {
                if !(1..=MAX_DEDICATED_MEDIA_LANES).contains(&cpus.len()) {
                    return Err(MediaRuntimeConfigError::InvalidDedicatedLaneCount {
                        lanes: cpus.len(),
                    });
                }
                let mut seen = BTreeSet::new();
                if let Some(cpu) = cpus.iter().flatten().find(|cpu| !seen.insert(**cpu)) {
                    return Err(MediaRuntimeConfigError::DuplicateLaneCpu { cpu: *cpu });
                }
                Ok(())
            }
        }
    }

    /// Worker threads for each lane on a host with `parallelism` CPUs.
    fn lane_worker_threads(
        &self,
        parallelism: usize,
    ) -> Result<Vec<usize>, MediaRuntimeConfigError> {
        self.validate()?;
        match &self.lanes {
            &MediaLanes::Shared {
                worker_threads,
                lanes,
            } => {
                let total = worker_threads
                    .unwrap_or_else(|| media_worker_threads_for(parallelism).max(lanes));
                Ok((0..lanes)
                    .map(|lane| total / lanes + usize::from(lane < total % lanes))
                    .collect())
            }
            MediaLanes::Dedicated { cpus } => Ok(vec![1; cpus.len()]),
        }
    }
}

/// Why a [`MediaRuntimeConfig`] was rejected.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MediaRuntimeConfigError {
    /// The shared lane count is outside `1..=MAX_SHARED_MEDIA_LANES`.
    #[error("media runtime lanes must be from 1 to {MAX_SHARED_MEDIA_LANES}, got {lanes}")]
    InvalidLaneCount {
        /// The rejected lane count.
        lanes: usize,
    },
    /// The explicit worker budget is outside `1..=MAX_MEDIA_WORKER_THREADS`.
    #[error(
        "media runtime worker threads must be from 1 to {MAX_MEDIA_WORKER_THREADS}, got {worker_threads}"
    )]
    InvalidWorkerThreads {
        /// The rejected worker-thread budget.
        worker_threads: usize,
    },
    /// Some lane would have no worker thread.
    #[error(
        "{lanes} media runtime lanes need at least {lanes} worker threads, got {worker_threads}"
    )]
    LanesExceedWorkerThreads {
        /// The requested lane count.
        lanes: usize,
        /// The requested total worker-thread budget.
        worker_threads: usize,
    },
    /// The dedicated lane count is outside `1..=MAX_DEDICATED_MEDIA_LANES`.
    #[error(
        "dedicated media runtime lanes must be from 1 to {MAX_DEDICATED_MEDIA_LANES}, got {lanes}"
    )]
    InvalidDedicatedLaneCount {
        /// The rejected lane count.
        lanes: usize,
    },
    /// Two dedicated lanes were given the same CPU.
    #[error("CPU {cpu} is assigned to more than one dedicated media runtime lane")]
    DuplicateLaneCpu {
        /// The repeated CPU.
        cpu: usize,
    },
    /// A different layout is already installed or already running.
    ///
    /// The media executor is process-wide and starts on first use, so it is
    /// configured once, before the first media task or transport.
    #[error("the media runtime is already configured as {active:?}")]
    AlreadyConfigured {
        /// The layout in effect.
        active: Box<MediaRuntimeConfig>,
    },
}

/// One lane's layout and placement counts, as reported by
/// [`media_runtime_snapshot`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MediaLaneSnapshot {
    /// Worker threads that poll this lane (always 1 for a dedicated lane).
    pub worker_threads: usize,
    /// The CPU this lane's thread is pinned to; `None` for a shared lane, an
    /// unpinned dedicated lane, or one whose pinning failed.
    pub pinned_cpu: Option<usize>,
    /// Media tasks placed on this lane by [`spawn_media_task`] and
    /// [`next_media_runtime_handle`].
    pub tasks_placed: u64,
    /// UDP transports whose sockets and receive tasks live on this lane.
    pub transports_placed: u64,
}

/// The running media executor, as reported by [`media_runtime_snapshot`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MediaRuntimeSnapshot {
    /// The layout in effect.
    pub config: MediaRuntimeConfig,
    /// Every lane, in placement order.
    pub lanes: Vec<MediaLaneSnapshot>,
}

static MEDIA_RUNTIME_CONFIG: OnceLock<MediaRuntimeConfig> = OnceLock::new();
static MEDIA_RUNTIME: OnceLock<MediaRuntimePool> = OnceLock::new();

thread_local! {
    static CURRENT_MEDIA_LANE: Cell<Option<usize>> = const { Cell::new(None) };
}

/// Install the media executor layout for this process.
///
/// Call this once at startup, before the first media task or transport; the
/// executor starts on first use with the default layout otherwise. Installing
/// a layout equal to the one in effect succeeds and changes nothing, so
/// several components may install the same configuration. A different layout
/// returns [`MediaRuntimeConfigError::AlreadyConfigured`].
pub fn install_media_runtime_config(
    config: MediaRuntimeConfig,
) -> Result<(), MediaRuntimeConfigError> {
    install_into(&MEDIA_RUNTIME_CONFIG, config)
}

fn install_into(
    slot: &OnceLock<MediaRuntimeConfig>,
    config: MediaRuntimeConfig,
) -> Result<(), MediaRuntimeConfigError> {
    config.validate()?;
    let active = slot.get_or_init(|| config.clone());
    if *active == config {
        Ok(())
    } else {
        Err(MediaRuntimeConfigError::AlreadyConfigured {
            active: Box::new(active.clone()),
        })
    }
}

/// Report the media executor's lanes, or `None` before it has started.
///
/// This never starts the executor. Counts are cumulative since start.
pub fn media_runtime_snapshot() -> Option<MediaRuntimeSnapshot> {
    let pool = MEDIA_RUNTIME.get()?;
    Some(snapshot_of(pool))
}

/// Start the media executor now and report its lanes.
///
/// The executor otherwise starts on first use. Starting it at boot, after
/// [`install_media_runtime_config`] and before the first call, creates (and
/// pins) every lane thread up front and lets the application log the layout
/// it got. Calling this again only reports.
pub fn start_media_runtime() -> MediaRuntimeSnapshot {
    snapshot_of(media_runtime())
}

fn snapshot_of(pool: &MediaRuntimePool) -> MediaRuntimeSnapshot {
    MediaRuntimeSnapshot {
        config: MEDIA_RUNTIME_CONFIG.get().cloned().unwrap_or_default(),
        lanes: pool.snapshot(),
    }
}

/// The index of the media lane that runs the calling thread, in placement
/// order; `None` on any thread that is not a media lane thread.
pub fn current_media_lane() -> Option<usize> {
    CURRENT_MEDIA_LANE.with(Cell::get)
}

/// Pin the calling thread to `cpus`; an empty list changes nothing.
///
/// Thread CPU affinity is applied on Linux only; other platforms return an
/// [`std::io::ErrorKind::Unsupported`] error. The kernel rejects CPUs outside
/// the process's CPU set.
pub fn pin_current_thread(cpus: &[usize]) -> std::io::Result<()> {
    if cpus.is_empty() {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        let capacity = std::mem::size_of::<libc::cpu_set_t>() * 8;
        if let Some(cpu) = cpus.iter().find(|cpu| **cpu >= capacity) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("CPU {cpu} exceeds the cpu_set_t capacity of {capacity}"),
            ));
        }
        // SAFETY: an all-zero `cpu_set_t` is the empty set, every index was
        // checked against its capacity above, and pid 0 names only the
        // calling thread.
        let rc = unsafe {
            let mut mask: libc::cpu_set_t = std::mem::zeroed();
            libc::CPU_ZERO(&mut mask);
            for &cpu in cpus {
                libc::CPU_SET(cpu, &mut mask);
            }
            libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &mask)
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "thread CPU affinity is only supported on Linux",
        ))
    }
}

struct MediaRuntimeLane {
    executor: LaneExecutor,
    worker_threads: usize,
    pinned_cpu: Option<usize>,
    tasks_placed: AtomicU64,
    transports_placed: AtomicU64,
}

enum LaneExecutor {
    Shared(Runtime),
    Dedicated(DedicatedLane),
}

impl MediaRuntimeLane {
    fn new(executor: LaneExecutor, worker_threads: usize, pinned_cpu: Option<usize>) -> Self {
        Self {
            executor,
            worker_threads,
            pinned_cpu,
            tasks_placed: AtomicU64::new(0),
            transports_placed: AtomicU64::new(0),
        }
    }

    fn handle(&self) -> &Handle {
        match &self.executor {
            LaneExecutor::Shared(runtime) => runtime.handle(),
            LaneExecutor::Dedicated(lane) => &lane.handle,
        }
    }
}

/// A current-thread runtime driven by its own thread until dropped.
struct DedicatedLane {
    handle: Handle,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl DedicatedLane {
    /// Start lane `lane`'s thread, pinned to `cpu` when given, and wait until
    /// its runtime exists. Returns the lane and the CPU actually applied.
    fn start(lane: usize, name: String, cpu: Option<usize>) -> (Self, Option<usize>) {
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let (shutdown, stopped) = tokio::sync::oneshot::channel::<()>();
        let thread = std::thread::Builder::new()
            .name(name)
            .spawn(move || {
                // Pin before the runtime exists so the driver's state is first
                // touched from the lane's own CPU.
                let pinned = match cpu {
                    Some(cpu) => match pin_current_thread(&[cpu]) {
                        Ok(()) => Some(cpu),
                        Err(error) => {
                            tracing::warn!(
                                lane,
                                cpu,
                                %error,
                                "media lane CPU pinning failed; the lane floats"
                            );
                            None
                        }
                    },
                    None => None,
                };
                CURRENT_MEDIA_LANE.with(|slot| slot.set(Some(lane)));
                let runtime = match Builder::new_current_thread().enable_all().build() {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                if ready_tx
                    .send(Ok((runtime.handle().clone(), pinned)))
                    .is_err()
                {
                    return;
                }
                runtime.block_on(async {
                    let _ = stopped.await;
                });
                // The runtime drops here, on its own thread, cancelling the
                // tasks the lane still owns.
            })
            .expect("start dedicated rvoip media lane thread");
        let (handle, pinned) = ready_rx
            .recv()
            .expect("dedicated rvoip media lane thread exited during startup")
            .expect("build dedicated rvoip media runtime");
        let lane = Self {
            handle,
            shutdown: Some(shutdown),
            thread: Some(thread),
        };
        (lane, pinned)
    }
}

impl Drop for DedicatedLane {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            // Joining from the lane's own thread would deadlock; its runtime
            // stops once the running task returns to `block_on`.
            if thread.thread().id() != std::thread::current().id() {
                let _ = thread.join();
            }
        }
    }
}

struct MediaRuntimePool {
    lanes: Vec<MediaRuntimeLane>,
    next_task: AtomicUsize,
    next_io: AtomicUsize,
}

fn lane_thread_name(lane: usize, lanes: usize) -> String {
    // A single lane keeps the historical thread name.
    if lanes > 1 {
        format!("rvoip-media-{lane}")
    } else {
        "rvoip-media".to_owned()
    }
}

impl MediaRuntimePool {
    fn from_config(config: &MediaRuntimeConfig, parallelism: usize) -> Self {
        match &config.lanes {
            MediaLanes::Dedicated { cpus } => {
                config
                    .validate()
                    .expect("installed media runtime configuration was validated");
                Self::dedicated(cpus)
            }
            MediaLanes::Shared { .. } => {
                let worker_counts = config
                    .lane_worker_threads(parallelism)
                    .expect("installed media runtime configuration was validated");
                Self::new(&worker_counts)
            }
        }
    }

    /// Shared lanes with the given worker-thread counts.
    fn new(worker_counts: &[usize]) -> Self {
        let lanes = worker_counts
            .iter()
            .enumerate()
            .map(|(lane, &worker_threads)| {
                let runtime = Builder::new_multi_thread()
                    .worker_threads(worker_threads)
                    .thread_name(lane_thread_name(lane, worker_counts.len()))
                    .on_thread_start(move || CURRENT_MEDIA_LANE.with(|slot| slot.set(Some(lane))))
                    .enable_all()
                    .build()
                    .expect("build dedicated rvoip media runtime");
                MediaRuntimeLane::new(LaneExecutor::Shared(runtime), worker_threads, None)
            })
            .collect();
        Self::from_lanes(lanes)
    }

    /// One current-thread lane per entry of `cpus`.
    fn dedicated(cpus: &[Option<usize>]) -> Self {
        let lanes = cpus
            .iter()
            .enumerate()
            .map(|(lane, &cpu)| {
                let (executor, pinned) =
                    DedicatedLane::start(lane, lane_thread_name(lane, cpus.len()), cpu);
                MediaRuntimeLane::new(LaneExecutor::Dedicated(executor), 1, pinned)
            })
            .collect();
        Self::from_lanes(lanes)
    }

    fn from_lanes(lanes: Vec<MediaRuntimeLane>) -> Self {
        Self {
            lanes,
            next_task: AtomicUsize::new(0),
            next_io: AtomicUsize::new(0),
        }
    }

    fn next_task_lane(&self) -> usize {
        // Used at task creation, never for each media packet.
        self.next_task.fetch_add(1, Ordering::Relaxed) % self.lanes.len()
    }

    fn next_io_lane(&self) -> usize {
        // Socket placement must not share the task counter: a constant number
        // of task spawns per call could repeatedly select one socket lane.
        self.next_io.fetch_add(1, Ordering::Relaxed) % self.lanes.len()
    }

    fn next_task_handle(&self) -> &Handle {
        let lane = &self.lanes[self.next_task_lane()];
        lane.tasks_placed.fetch_add(1, Ordering::Relaxed);
        lane.handle()
    }

    fn next_io_handle(&self) -> &Handle {
        let lane = &self.lanes[self.next_io_lane()];
        lane.transports_placed.fetch_add(1, Ordering::Relaxed);
        lane.handle()
    }

    fn snapshot(&self) -> Vec<MediaLaneSnapshot> {
        self.lanes
            .iter()
            .map(|lane| MediaLaneSnapshot {
                worker_threads: lane.worker_threads,
                pinned_cpu: lane.pinned_cpu,
                tasks_placed: lane.tasks_placed.load(Ordering::Relaxed),
                transports_placed: lane.transports_placed.load(Ordering::Relaxed),
            })
            .collect()
    }
}

fn media_worker_threads_for(parallelism: usize) -> usize {
    // Media polling is isolated from signaling, so using half of the host is a
    // real media-plane capacity allocation rather than competition with SIP
    // transaction timers. Eight workers are sufficient for the supported
    // dense-call profile while retaining a bounded executor on larger hosts.
    parallelism.div_ceil(2).clamp(2, 8)
}

fn available_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(4)
}

fn media_runtime() -> &'static MediaRuntimePool {
    MEDIA_RUNTIME.get_or_init(|| {
        let config = MEDIA_RUNTIME_CONFIG.get_or_init(MediaRuntimeConfig::default);
        MediaRuntimePool::from_config(config, available_parallelism())
    })
}

#[cfg(test)]
thread_local! {
    static SPAWN_ON_CURRENT_RUNTIME: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn spawn_on_current_runtime() -> bool {
    SPAWN_ON_CURRENT_RUNTIME.with(std::cell::Cell::get)
}

/// While alive, media tasks and transports created from this thread run on
/// the caller's runtime instead of the dedicated media executor, so a
/// `#[tokio::test(start_paused = true)]` clock drives their timers too.
#[cfg(test)]
pub(crate) struct MediaTasksOnCurrentRuntime(());

#[cfg(test)]
impl MediaTasksOnCurrentRuntime {
    pub(crate) fn enter() -> Self {
        SPAWN_ON_CURRENT_RUNTIME.with(|flag| flag.set(true));
        Self(())
    }
}

#[cfg(test)]
impl Drop for MediaTasksOnCurrentRuntime {
    fn drop(&mut self) {
        SPAWN_ON_CURRENT_RUNTIME.with(|flag| flag.set(false));
    }
}

/// Select a media lane for an externally tracked task.
///
/// The process-wide pool owns its runtimes; the caller's `JoinHandle` or
/// `JoinSet` retains shutdown and drain authority. Lanes are chosen round
/// robin, once per call.
#[doc(hidden)]
pub fn next_media_runtime_handle() -> Handle {
    #[cfg(test)]
    if spawn_on_current_runtime() {
        return Handle::current();
    }
    media_runtime().next_task_handle().clone()
}

/// Select the lane for one UDP transport's sockets and receive tasks.
pub(crate) fn next_media_io_runtime_handle() -> Handle {
    #[cfg(test)]
    if spawn_on_current_runtime() {
        return Handle::current();
    }
    media_runtime().next_io_handle().clone()
}

/// Spawn a lifecycle-owned media task without consuming signaling-executor
/// scheduling budget. The returned handle remains the sole join/cancel owner.
#[doc(hidden)]
pub fn spawn_media_task<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    #[cfg(test)]
    if spawn_on_current_runtime() {
        return tokio::spawn(future);
    }
    media_runtime().next_task_handle().spawn(future)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn shared(worker_threads: Option<usize>, lanes: usize) -> MediaRuntimeConfig {
        MediaRuntimeConfig::new(MediaLanes::Shared {
            worker_threads,
            lanes,
        })
    }

    #[test]
    fn media_worker_policy_scales_for_dense_hosts_and_remains_bounded() {
        assert_eq!(media_worker_threads_for(1), 2);
        assert_eq!(media_worker_threads_for(4), 2);
        assert_eq!(media_worker_threads_for(8), 4);
        assert_eq!(media_worker_threads_for(16), 8);
        assert_eq!(media_worker_threads_for(128), 8);
    }

    #[test]
    fn default_layout_is_one_lane_with_the_existing_worker_policy() {
        let config = MediaRuntimeConfig::default();
        assert_eq!(config, shared(None, 1));
        for parallelism in [1, 4, 8, 16, 32, 128] {
            assert_eq!(
                config.lane_worker_threads(parallelism).unwrap(),
                vec![media_worker_threads_for(parallelism)],
                "parallelism {parallelism}"
            );
        }
    }

    #[test]
    fn lane_configuration_preserves_total_workers_and_rejects_oversubscription() {
        assert_eq!(
            shared(Some(16), 1).lane_worker_threads(64).unwrap(),
            vec![16]
        );
        assert_eq!(
            shared(Some(48), 4).lane_worker_threads(64).unwrap(),
            vec![12; 4]
        );
        assert_eq!(
            shared(Some(17), 4).lane_worker_threads(64).unwrap(),
            vec![5, 4, 4, 4]
        );
        // The built-in budget is split, not multiplied, by lanes...
        assert_eq!(shared(None, 2).lane_worker_threads(32).unwrap(), vec![4, 4]);
        // ...and raised so a small host still has one worker per lane.
        assert_eq!(shared(None, 4).lane_worker_threads(2).unwrap(), vec![1; 4]);
        for lanes in [0, MAX_SHARED_MEDIA_LANES + 1] {
            assert_eq!(
                shared(None, lanes).validate(),
                Err(MediaRuntimeConfigError::InvalidLaneCount { lanes })
            );
        }
        assert_eq!(
            shared(Some(2), 4).validate(),
            Err(MediaRuntimeConfigError::LanesExceedWorkerThreads {
                lanes: 4,
                worker_threads: 2
            })
        );
    }

    #[test]
    fn explicit_media_worker_budget_is_bounded_and_preserves_the_default() {
        assert_eq!(shared(None, 1).lane_worker_threads(64).unwrap(), vec![8]);
        for workers in [1, 32, MAX_MEDIA_WORKER_THREADS] {
            assert_eq!(
                shared(Some(workers), 1).lane_worker_threads(64).unwrap(),
                vec![workers]
            );
        }
        for worker_threads in [0, MAX_MEDIA_WORKER_THREADS + 1] {
            assert_eq!(
                shared(Some(worker_threads), 1).validate(),
                Err(MediaRuntimeConfigError::InvalidWorkerThreads { worker_threads })
            );
        }
    }

    #[test]
    fn dedicated_layout_needs_one_to_max_lanes_and_distinct_cpus() {
        let dedicated =
            |cpus: Vec<Option<usize>>| MediaRuntimeConfig::new(MediaLanes::Dedicated { cpus });
        assert_eq!(
            MediaLanes::dedicated(2),
            MediaLanes::Dedicated {
                cpus: vec![None, None]
            }
        );
        assert_eq!(
            MediaLanes::pinned([3, 1]),
            MediaLanes::Dedicated {
                cpus: vec![Some(3), Some(1)]
            }
        );
        // The thread count is the lane count, whatever the host.
        let config = MediaRuntimeConfig::new(MediaLanes::pinned([3, 1, 2]));
        assert_eq!(config.lane_worker_threads(64).unwrap(), vec![1; 3]);
        dedicated(vec![Some(0), None, None, Some(7)])
            .validate()
            .unwrap();
        dedicated(vec![None; MAX_DEDICATED_MEDIA_LANES])
            .validate()
            .unwrap();
        for lanes in [0, MAX_DEDICATED_MEDIA_LANES + 1] {
            assert_eq!(
                dedicated(vec![None; lanes]).validate(),
                Err(MediaRuntimeConfigError::InvalidDedicatedLaneCount { lanes })
            );
        }
        assert_eq!(
            dedicated(vec![Some(4), None, Some(4)]).validate(),
            Err(MediaRuntimeConfigError::DuplicateLaneCpu { cpu: 4 })
        );
    }

    #[test]
    fn install_accepts_an_equal_layout_and_rejects_a_different_one() {
        let slot = OnceLock::new();
        assert_eq!(
            install_into(&slot, shared(None, 0)),
            Err(MediaRuntimeConfigError::InvalidLaneCount { lanes: 0 }),
        );
        assert!(slot.get().is_none(), "an invalid layout installs nothing");
        install_into(&slot, shared(Some(8), 2)).unwrap();
        install_into(&slot, shared(Some(8), 2)).unwrap();
        let error = install_into(&slot, MediaRuntimeConfig::default()).unwrap_err();
        assert_eq!(
            error,
            MediaRuntimeConfigError::AlreadyConfigured {
                active: Box::new(shared(Some(8), 2)),
            }
        );
        assert_eq!(slot.get(), Some(&shared(Some(8), 2)));

        // A runtime that started without an install runs the default layout,
        // so installing the default afterwards is still accepted.
        let started = OnceLock::new();
        started.get_or_init(MediaRuntimeConfig::default);
        install_into(&started, MediaRuntimeConfig::default()).unwrap();
        assert!(install_into(&started, shared(None, 2)).is_err());
    }

    #[test]
    fn install_once_covers_dedicated_layouts_and_their_cpus() {
        let pinned = || MediaRuntimeConfig::new(MediaLanes::pinned([2, 3]));
        let slot = OnceLock::new();
        assert_eq!(
            install_into(&slot, MediaRuntimeConfig::new(MediaLanes::pinned([2, 2]))),
            Err(MediaRuntimeConfigError::DuplicateLaneCpu { cpu: 2 }),
        );
        assert!(slot.get().is_none(), "an invalid layout installs nothing");
        install_into(&slot, pinned()).unwrap();
        install_into(&slot, pinned()).unwrap();
        // Same lane count, different placement: still a different layout.
        for other in [
            MediaRuntimeConfig::new(MediaLanes::pinned([3, 2])),
            MediaRuntimeConfig::new(MediaLanes::dedicated(2)),
            shared(Some(2), 2),
        ] {
            assert_eq!(
                install_into(&slot, other),
                Err(MediaRuntimeConfigError::AlreadyConfigured {
                    active: Box::new(pinned()),
                })
            );
        }
        assert_eq!(slot.get(), Some(&pinned()));
    }

    #[test]
    fn independent_media_timer_lane_progresses_and_owned_shutdown_joins() {
        // Own/drop the runtimes outside their async contexts.
        let pool = MediaRuntimePool::new(&[1, 1]);
        let root = Builder::new_current_thread().enable_all().build().unwrap();
        root.block_on(async {
            let mut tasks = tokio::task::JoinSet::new();
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
            // Block lane 0's only worker...
            tasks.spawn_on(
                async move {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    0
                },
                pool.next_task_handle(),
            );
            started_rx.await.unwrap();
            // ...and lane 1's timer driver must still make progress.
            tasks.spawn_on(
                async {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    17
                },
                pool.next_task_handle(),
            );
            let progressed = tokio::time::timeout(Duration::from_secs(1), tasks.join_next()).await;
            release_tx.send(()).unwrap();
            assert_eq!(progressed.unwrap().unwrap().unwrap(), 17);
            assert_eq!(tasks.join_next().await.unwrap().unwrap(), 0);
            assert!(tasks.is_empty());
        });
    }

    #[test]
    fn shared_lane_threads_report_their_lane() {
        let pool = MediaRuntimePool::new(&[2, 1]);
        let root = Builder::new_current_thread().enable_all().build().unwrap();
        root.block_on(async {
            assert_eq!(current_media_lane(), None, "not a media thread");
            let first = pool
                .next_task_handle()
                .spawn(async { current_media_lane() });
            let second = pool
                .next_task_handle()
                .spawn(async { current_media_lane() });
            assert_eq!(first.await.unwrap(), Some(0));
            assert_eq!(second.await.unwrap(), Some(1));
        });
    }

    #[test]
    fn current_thread_owners_keep_independent_timers_and_lane_identity() {
        let pool = MediaRuntimePool::dedicated(&[None, None]);
        let root = Builder::new_current_thread().enable_all().build().unwrap();
        root.block_on(async {
            let probe = || async { (std::thread::current().id(), current_media_lane()) };
            // Each lane is one thread: the same lane always answers from the
            // same thread, and two lanes never share one.
            let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let blocked = pool.next_task_handle().spawn(async move {
                started_tx.send(std::thread::current().id()).unwrap();
                // Stall lane 0's only thread.
                release_rx.recv().unwrap();
                current_media_lane()
            });
            let lane0_thread = started_rx.await.unwrap();
            // Lane 1's timer still fires while lane 0 is stalled.
            let lane1 = pool.next_task_handle().spawn(async move {
                tokio::time::sleep(Duration::from_millis(5)).await;
                probe().await
            });
            let (lane1_thread, lane1_id) = tokio::time::timeout(Duration::from_secs(1), lane1)
                .await
                .expect("lane 1 timer progresses independently")
                .unwrap();
            release_tx.send(()).unwrap();
            assert_eq!(blocked.await.unwrap(), Some(0));
            assert_eq!(lane1_id, Some(1));
            assert_ne!(lane0_thread, lane1_thread);
            assert_ne!(lane0_thread, std::thread::current().id());

            let again = pool.next_task_handle().spawn(probe()).await.unwrap();
            assert_eq!(again, (lane0_thread, Some(0)));
        });
        let snapshot = pool.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert!(snapshot
            .iter()
            .all(|lane| lane.worker_threads == 1 && lane.pinned_cpu.is_none()));
        assert_eq!(
            snapshot
                .iter()
                .map(|lane| lane.tasks_placed)
                .collect::<Vec<_>>(),
            [2, 1]
        );
    }

    #[test]
    fn dropping_current_thread_pool_acknowledges_owned_task_cancellation() {
        use std::sync::Arc;
        struct Probe(Arc<AtomicUsize>);
        impl Drop for Probe {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(AtomicUsize::new(0));
        let pool = MediaRuntimePool::dedicated(&[None]);
        let root = Builder::new_current_thread().enable_all().build().unwrap();
        let (started, ready) = tokio::sync::oneshot::channel();
        let owned = Arc::clone(&dropped);
        let pending = pool.next_task_handle().spawn(async move {
            let _probe = Probe(owned);
            started.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        root.block_on(async {
            tokio::time::timeout(Duration::from_secs(1), ready)
                .await
                .unwrap()
                .unwrap();
        });
        // Dropping the pool stops and joins the lane thread, which drops the
        // task it still owns.
        drop(pool);
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        assert!(root.block_on(pending).unwrap_err().is_cancelled());
    }

    #[test]
    fn dedicated_lane_reactor_serves_its_own_sockets() {
        let pool = MediaRuntimePool::dedicated(&[None]);
        let root = Builder::new_current_thread().enable_all().build().unwrap();
        root.block_on(async {
            let lane = pool.next_io_handle().clone();
            let sender = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
            // Bound on the lane: its readiness is the lane's reactor's.
            let receiver = lane
                .spawn(async { tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap() })
                .await
                .unwrap();
            let address = receiver.local_addr().unwrap();
            let received = lane.spawn(async move {
                let mut payload = [0; 16];
                let (length, _) = receiver.recv_from(&mut payload).await.unwrap();
                (payload[..length].to_vec(), current_media_lane())
            });
            sender.send_to(b"lane-media", address).await.unwrap();
            let (payload, lane_id) = tokio::time::timeout(Duration::from_secs(1), received)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(payload, b"lane-media");
            assert_eq!(lane_id, Some(0));
        });
        assert_eq!(pool.snapshot()[0].transports_placed, 1);
    }

    /// CPUs this thread may run on, or `None` when the mask is unreadable.
    #[cfg(target_os = "linux")]
    fn allowed_cpus() -> Option<Vec<usize>> {
        // SAFETY: an all-zero `cpu_set_t` is the empty set; the kernel fills
        // at most `size_of::<cpu_set_t>()` bytes and `CPU_ISSET` reads only
        // indexes below its capacity.
        unsafe {
            let mut mask: libc::cpu_set_t = std::mem::zeroed();
            if libc::sched_getaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &mut mask) != 0 {
                return None;
            }
            let capacity = std::mem::size_of::<libc::cpu_set_t>() * 8;
            Some(
                (0..capacity)
                    .filter(|cpu| libc::CPU_ISSET(*cpu, &mask))
                    .collect(),
            )
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn dedicated_lane_pins_its_thread_to_the_configured_cpu() {
        let Some(allowed) = allowed_cpus() else {
            eprintln!("skipping: sched_getaffinity is not permitted here");
            return;
        };
        // The highest permitted CPU, so the lane is unlikely to share the
        // test thread's CPU by accident.
        let cpu = *allowed.last().expect("at least one permitted CPU");
        let pool = MediaRuntimePool::dedicated(&[Some(cpu), None]);
        let snapshot = pool.snapshot();
        if snapshot[0].pinned_cpu.is_none() {
            // A sandbox may refuse sched_setaffinity; the lane must still run.
            eprintln!("skipping pinning assertions: the host refused CPU {cpu}");
        } else {
            assert_eq!(snapshot[0].pinned_cpu, Some(cpu));
        }
        assert_eq!(snapshot[1].pinned_cpu, None);
        let root = Builder::new_current_thread().enable_all().build().unwrap();
        let (mask, lane) = root.block_on(async {
            pool.next_task_handle()
                .spawn(async { (allowed_cpus(), current_media_lane()) })
                .await
                .unwrap()
        });
        assert_eq!(lane, Some(0));
        if snapshot[0].pinned_cpu.is_some() {
            assert_eq!(mask, Some(vec![cpu]), "lane 0 runs only on CPU {cpu}");
        }
        // The unpinned lane keeps the process's mask.
        let (floating, _) = root.block_on(async {
            pool.next_task_handle()
                .spawn(async { (allowed_cpus(), current_media_lane()) })
                .await
                .unwrap()
        });
        assert_eq!(floating, Some(allowed));
    }

    #[test]
    fn a_cpu_the_host_cannot_apply_leaves_the_lane_floating() {
        // Beyond any cpu_set_t: rejected without touching the thread.
        let pool = MediaRuntimePool::dedicated(&[Some(1 << 20)]);
        assert_eq!(pool.snapshot()[0].pinned_cpu, None);
        let root = Builder::new_current_thread().enable_all().build().unwrap();
        let lane = root.block_on(async {
            pool.next_task_handle()
                .spawn(async { current_media_lane() })
                .await
                .unwrap()
        });
        assert_eq!(lane, Some(0), "the lane still runs its tasks");
        assert!(pin_current_thread(&[]).is_ok(), "an empty list is a no-op");
    }

    #[test]
    fn socket_lane_round_robin_is_independent_of_other_task_creation() {
        let pool = MediaRuntimePool::new(&[1, 1, 1]);
        let mut lanes = Vec::new();
        for _ in 0..6 {
            lanes.push(pool.next_io_lane());
            // A fixed number of task spawns per transport must not alias
            // every transport onto one lane.
            pool.next_task_lane();
            pool.next_task_lane();
            pool.next_task_lane();
        }
        assert_eq!(lanes, [0, 1, 2, 0, 1, 2]);

        let root = Builder::new_current_thread().enable_all().build().unwrap();
        root.block_on(async {
            let thread = |handle: &Handle| handle.spawn(async { std::thread::current().id() });
            let first = thread(pool.next_io_handle()).await.unwrap();
            let _other_task = pool.next_task_handle().spawn(async {}).await.unwrap();
            let second = thread(pool.next_io_handle()).await.unwrap();
            assert_ne!(first, second, "consecutive transports use different lanes");
        });
        let counts = pool
            .snapshot()
            .into_iter()
            .map(|lane| (lane.worker_threads, lane.transports_placed))
            .collect::<Vec<_>>();
        // Only selections through the handle count as placements.
        assert_eq!(counts, [(1, 1), (1, 1), (1, 0)]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn media_task_runs_outside_signaling_executor() {
        let signaling_thread = std::thread::current().id();
        let media_thread = spawn_media_task(async { std::thread::current().id() })
            .await
            .expect("media task");
        assert_ne!(signaling_thread, media_thread);
    }
}
