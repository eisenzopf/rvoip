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
//! Each UDP transport selects one lane when it is created. Its sockets are
//! registered on that lane's reactor and its receive tasks run on that lane,
//! so readiness for a media socket never wakes the signaling runtime that
//! created it. Socket placement uses its own round-robin counter, independent
//! of other task creation, so a fixed number of task spawns per call cannot
//! alias every socket onto one lane.

use std::future::Future;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::OnceLock;

use tokio::runtime::{Builder, Handle, Runtime};
use tokio::task::JoinHandle;

/// Largest number of shared lanes [`MediaLanes::Shared`] accepts.
pub const MAX_SHARED_MEDIA_LANES: usize = 8;

/// Largest explicit media worker-thread budget [`MediaLanes::Shared`] accepts.
pub const MAX_MEDIA_WORKER_THREADS: usize = 64;

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
        match self.lanes {
            MediaLanes::Shared {
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
        }
    }

    /// Worker threads for each lane on a host with `parallelism` CPUs.
    fn lane_worker_threads(
        &self,
        parallelism: usize,
    ) -> Result<Vec<usize>, MediaRuntimeConfigError> {
        self.validate()?;
        match self.lanes {
            MediaLanes::Shared {
                worker_threads,
                lanes,
            } => {
                let total = worker_threads
                    .unwrap_or_else(|| media_worker_threads_for(parallelism).max(lanes));
                Ok((0..lanes)
                    .map(|lane| total / lanes + usize::from(lane < total % lanes))
                    .collect())
            }
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
    /// Worker threads that poll this lane.
    pub worker_threads: usize,
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
    Some(MediaRuntimeSnapshot {
        config: MEDIA_RUNTIME_CONFIG.get().cloned().unwrap_or_default(),
        lanes: pool.snapshot(),
    })
}

struct MediaRuntimeLane {
    runtime: Runtime,
    worker_threads: usize,
    tasks_placed: AtomicU64,
    transports_placed: AtomicU64,
}

struct MediaRuntimePool {
    lanes: Vec<MediaRuntimeLane>,
    next_task: AtomicUsize,
    next_io: AtomicUsize,
}

impl MediaRuntimePool {
    fn new(worker_counts: &[usize]) -> Self {
        let named_lanes = worker_counts.len() > 1;
        let lanes = worker_counts
            .iter()
            .enumerate()
            .map(|(lane, &worker_threads)| {
                // A single lane keeps the historical thread name.
                let name = if named_lanes {
                    format!("rvoip-media-{lane}")
                } else {
                    "rvoip-media".to_owned()
                };
                MediaRuntimeLane {
                    runtime: Builder::new_multi_thread()
                        .worker_threads(worker_threads)
                        .thread_name(name)
                        .enable_all()
                        .build()
                        .expect("build dedicated rvoip media runtime"),
                    worker_threads,
                    tasks_placed: AtomicU64::new(0),
                    transports_placed: AtomicU64::new(0),
                }
            })
            .collect();
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
        lane.runtime.handle()
    }

    fn next_io_handle(&self) -> &Handle {
        let lane = &self.lanes[self.next_io_lane()];
        lane.transports_placed.fetch_add(1, Ordering::Relaxed);
        lane.runtime.handle()
    }

    fn snapshot(&self) -> Vec<MediaLaneSnapshot> {
        self.lanes
            .iter()
            .map(|lane| MediaLaneSnapshot {
                worker_threads: lane.worker_threads,
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
        let worker_counts = config
            .lane_worker_threads(available_parallelism())
            .expect("installed media runtime configuration was validated");
        MediaRuntimePool::new(&worker_counts)
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
