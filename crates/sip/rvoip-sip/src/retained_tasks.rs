use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::sync::Notify;

const CLOSED_BIT: usize = 1 << (usize::BITS - 1);
const COUNT_MASK: usize = !CLOSED_BIT;

/// Counts owner-managed tasks through completion without aborting them.
///
/// The high bit in `state` is a strict external-admission fence and the
/// remaining bits are the active task count. Packing them into one atomic
/// makes admission and close linearizable without putting a global mutex on
/// lifecycle hot paths.
///
/// A retained parent may still use `spawn_child` while draining. Because the
/// parent keeps the count nonzero until after child registration, `wait_idle`
/// cannot miss a child admitted concurrently with close.
pub(crate) struct RetainedTasks {
    runtime: Handle,
    state: AtomicUsize,
    panicked: AtomicBool,
    idle: Notify,
}

impl RetainedTasks {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            runtime: Handle::current(),
            state: AtomicUsize::new(0),
            panicked: AtomicBool::new(false),
            idle: Notify::new(),
        })
    }

    pub(crate) fn spawn(
        self: &Arc<Self>,
        future: impl Future<Output = ()> + Send + 'static,
    ) -> bool {
        let mut current = self.state.load(Ordering::Acquire);
        loop {
            if current & CLOSED_BIT != 0 {
                return false;
            }
            assert_ne!(
                current & COUNT_MASK,
                COUNT_MASK,
                "retained task count overflow"
            );
            match self.state.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(observed) => current = observed,
            }
        }
        self.spawn_counted(future);
        true
    }

    /// Retain cleanup spawned by an already-retained parent after close.
    ///
    /// The caller must itself be retained until this method returns. That
    /// parent count prevents `wait_idle` from observing a transient zero while
    /// the child registration is being installed.
    pub(crate) fn spawn_child(self: &Arc<Self>, future: impl Future<Output = ()> + Send + 'static) {
        let mut current = self.state.load(Ordering::Acquire);
        loop {
            assert_ne!(
                current & COUNT_MASK,
                0,
                "spawn_child requires a retained parent"
            );
            assert_ne!(
                current & COUNT_MASK,
                COUNT_MASK,
                "retained task count overflow"
            );
            match self.state.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(observed) => current = observed,
            }
        }
        self.spawn_counted(future);
    }

    fn spawn_counted(self: &Arc<Self>, future: impl Future<Output = ()> + Send + 'static) {
        let completion = RetainedTaskCompletion {
            tasks: Arc::clone(self),
            completed_normally: false,
        };
        self.runtime.spawn(async move {
            let mut completion = completion;
            future.await;
            completion.completed_normally = true;
        });
    }

    pub(crate) fn close(&self) {
        let previous = self.state.fetch_or(CLOSED_BIT, Ordering::AcqRel);
        if previous & COUNT_MASK == 0 {
            self.idle.notify_waiters();
        }
    }

    pub(crate) fn count(&self) -> usize {
        self.state.load(Ordering::Acquire) & COUNT_MASK
    }

    pub(crate) fn panicked(&self) -> bool {
        self.panicked.load(Ordering::Acquire)
    }

    pub(crate) async fn wait_idle(&self) {
        loop {
            let notified = self.idle.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.count() == 0 {
                return;
            }
            notified.await;
        }
    }

    fn finish_one(&self) {
        let previous = self.state.fetch_sub(1, Ordering::AcqRel);
        debug_assert_ne!(previous & COUNT_MASK, 0);
        if previous & COUNT_MASK == 1 {
            self.idle.notify_waiters();
        }
    }
}

struct RetainedTaskCompletion {
    tasks: Arc<RetainedTasks>,
    completed_normally: bool,
}

impl Drop for RetainedTaskCompletion {
    fn drop(&mut self) {
        if !self.completed_normally {
            self.tasks.panicked.store(true, Ordering::Release);
        }
        self.tasks.finish_one();
    }
}

#[cfg(test)]
mod tests {
    use super::RetainedTasks;
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn retained_parent_can_register_child_during_close() {
        let tasks = RetainedTasks::new();
        let (parent_started_tx, parent_started_rx) = tokio::sync::oneshot::channel();
        let (allow_child_tx, allow_child_rx) = tokio::sync::oneshot::channel();
        let (child_done_tx, child_done_rx) = tokio::sync::oneshot::channel();
        let parent_tasks = Arc::clone(&tasks);
        assert!(tasks.spawn(async move {
            let _ = parent_started_tx.send(());
            let _ = allow_child_rx.await;
            parent_tasks.spawn_child(async move {
                tokio::task::yield_now().await;
                let _ = child_done_tx.send(());
            });
        }));

        parent_started_rx.await.expect("retained parent started");
        tasks.close();
        let _ = allow_child_tx.send(());
        tokio::time::timeout(Duration::from_secs(1), tasks.wait_idle())
            .await
            .expect("parent and child drained");
        child_done_rx.await.expect("child ran before idle");
        assert_eq!(tasks.count(), 0);
        assert!(!tasks.panicked());
    }

    #[tokio::test]
    async fn close_rejects_external_admission() {
        let tasks = RetainedTasks::new();
        tasks.close();
        assert!(!tasks.spawn(async {}));
        tasks.wait_idle().await;
        assert_eq!(tasks.count(), 0);
        assert!(!tasks.panicked());
    }

    #[tokio::test]
    async fn cooperative_cancellation_is_not_a_panic() {
        let tasks = RetainedTasks::new();
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        assert!(tasks.spawn(async move {
            let _ = cancel_rx.await;
        }));
        tasks.close();
        let _ = cancel_tx.send(());
        tokio::time::timeout(Duration::from_secs(1), tasks.wait_idle())
            .await
            .expect("cooperatively cancelled task drained");
        assert_eq!(tasks.count(), 0);
        assert!(!tasks.panicked());
    }

    #[tokio::test]
    async fn task_panic_is_reported_after_drain() {
        let tasks = RetainedTasks::new();
        assert!(tasks.spawn(async {
            panic!("retained task panic canary");
        }));
        tasks.close();
        tokio::time::timeout(Duration::from_secs(1), tasks.wait_idle())
            .await
            .expect("panicked task released its retained count");
        assert_eq!(tasks.count(), 0);
        assert!(tasks.panicked());
    }

    #[tokio::test]
    async fn work_spawned_off_runtime_runs_and_drains() {
        let tasks = RetainedTasks::new();
        let thread_tasks = Arc::clone(&tasks);
        let (ran_tx, ran_rx) = tokio::sync::oneshot::channel();

        std::thread::spawn(move || {
            assert!(tokio::runtime::Handle::try_current().is_err());
            assert!(thread_tasks.spawn(async move {
                let _ = ran_tx.send(());
            }));
        })
        .join()
        .expect("off-runtime retained task admission must not panic");

        ran_rx.await.expect("off-runtime retained task ran");
        tasks.close();
        tokio::time::timeout(Duration::from_secs(1), tasks.wait_idle())
            .await
            .expect("off-runtime retained task drained");
        assert_eq!(tasks.count(), 0);
        assert!(!tasks.panicked());
    }
}
