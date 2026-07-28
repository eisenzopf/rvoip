//! Runtime isolation for RTP/media-plane tasks.
//!
//! A dense media workload can have thousands of 20 ms timers and socket
//! continuations ready at once. Running those continuations on the caller's
//! signaling executor allows valid RTP load to delay SIP transaction timers
//! and response handling. Media remains lifecycle-owned by its `JoinHandle`,
//! but its polling budget is supplied by this dedicated executor.

use std::future::Future;
use std::sync::OnceLock;

use tokio::runtime::{Builder, Runtime};
use tokio::task::JoinHandle;

static MEDIA_RUNTIME: OnceLock<Runtime> = OnceLock::new();

fn media_worker_threads_for(parallelism: usize) -> usize {
    // Media polling is isolated from signaling, so using half of the host is a
    // real media-plane capacity allocation rather than competition with SIP
    // transaction timers. Eight workers are sufficient for the supported
    // dense-call profile while retaining a bounded executor on larger hosts.
    parallelism.div_ceil(2).clamp(2, 8)
}

fn media_worker_threads() -> usize {
    media_worker_threads_for(
        std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(4),
    )
}

fn media_runtime() -> &'static Runtime {
    MEDIA_RUNTIME.get_or_init(|| {
        Builder::new_multi_thread()
            .worker_threads(media_worker_threads())
            .thread_name("rvoip-media")
            .enable_all()
            .build()
            .expect("build dedicated rvoip media runtime")
    })
}

/// Spawn a lifecycle-owned media task without consuming signaling-executor
/// scheduling budget. The returned handle remains the sole join/cancel owner.
#[doc(hidden)]
pub fn spawn_media_task<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    media_runtime().spawn(future)
}

#[cfg(test)]
mod tests {
    #[test]
    fn media_worker_policy_scales_for_dense_hosts_and_remains_bounded() {
        assert_eq!(super::media_worker_threads_for(1), 2);
        assert_eq!(super::media_worker_threads_for(4), 2);
        assert_eq!(super::media_worker_threads_for(8), 4);
        assert_eq!(super::media_worker_threads_for(16), 8);
        assert_eq!(super::media_worker_threads_for(128), 8);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn media_task_runs_outside_signaling_executor() {
        let signaling_thread = std::thread::current().id();
        let media_thread = super::spawn_media_task(async { std::thread::current().id() })
            .await
            .expect("media task");
        assert_ne!(signaling_thread, media_thread);
    }
}
