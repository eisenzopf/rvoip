//! Session cleanup and resource management

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::interval;
use serde::{Serialize, Deserialize};
use tracing::{info, debug};
use crate::state_table::CallState;
use crate::state_table::{SessionId, DialogId, MediaSessionId, CallId};
use super::{SessionStore, SessionState};

/// Configuration for automatic cleanup
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupConfig {
    /// How often to run cleanup
    pub interval: Duration,

    /// TTL for terminated sessions
    pub terminated_ttl: Duration,

    /// TTL for failed sessions
    pub failed_ttl: Duration,

    /// Maximum idle time before cleanup
    pub max_idle_time: Duration,

    /// Maximum session age
    pub max_session_age: Duration,

    /// Enable automatic cleanup
    pub enabled: bool,

    /// Maximum memory usage before aggressive cleanup (in bytes)
    pub max_memory_bytes: Option<usize>,

    /// Maximum number of sessions before cleanup
    pub max_sessions: Option<usize>,
}

impl Default for CleanupConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(60),
            terminated_ttl: Duration::from_secs(300),
            failed_ttl: Duration::from_secs(600),
            max_idle_time: Duration::from_secs(3600),
            max_session_age: Duration::from_secs(86400),
            enabled: true,
            max_memory_bytes: None,
            max_sessions: None,
        }
    }
}

/// Statistics from cleanup run
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CleanupStats {
    pub sessions_checked: usize,
    pub total_removed: usize,
    pub terminated_removed: usize,
    pub failed_removed: usize,
    pub idle_removed: usize,
    pub aged_removed: usize,
    pub memory_pressure_removed: usize,
    pub active_preserved: usize,
    pub duration: Duration,
}

/// Resource limits for the session store
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Maximum number of concurrent sessions
    pub max_sessions: Option<usize>,

    /// Maximum memory per session (in bytes)
    pub max_memory_per_session: Option<usize>,

    /// Maximum history entries per session
    pub max_history_per_session: usize,

    /// Rate limit for new sessions (per second)
    pub max_sessions_per_second: Option<f64>,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_sessions: None,
            max_memory_per_session: Some(10 * 1024 * 1024),
            max_history_per_session: 100,
            max_sessions_per_second: None,
        }
    }
}

/// Overall resource usage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUsageReport {
    pub total_sessions: usize,
    pub active_sessions: usize,
    pub idle_sessions: usize,
    pub total_memory_bytes: usize,
    pub average_memory_per_session: usize,
    pub history_entries_total: usize,
    pub oldest_session_age: Option<Duration>,
    pub most_idle_session: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
enum RemovalReason {
    Terminated,
    Failed,
    Idle,
    Aged,
    MemoryPressure,
}

impl SessionStore {
    /// Run cleanup on sessions based on config
    pub async fn cleanup_sessions(&self, config: CleanupConfig) -> CleanupStats {
        let start = Instant::now();
        let mut stats = CleanupStats::default();

        if !config.enabled {
            return stats;
        }

        let sessions = self.sessions.read().await;
        let total_sessions = sessions.len();
        stats.sessions_checked = total_sessions;

        let mut to_remove = Vec::new();
        let now = Instant::now();

        for (id, session) in sessions.iter() {
            let session_age = now.duration_since(session.created_at);
            let time_in_state = now.duration_since(session.entered_state_at);

            let should_remove = match session.call_state {
                CallState::Terminated => {
                    if time_in_state > config.terminated_ttl {
                        stats.terminated_removed += 1;
                        true
                    } else {
                        false
                    }
                }
                CallState::Failed(_) => {
                    if time_in_state > config.failed_ttl {
                        stats.failed_removed += 1;
                        true
                    } else {
                        false
                    }
                }
                CallState::Active | CallState::OnHold | CallState::Bridged => {
                    stats.active_preserved += 1;
                    false
                }
                _ => {
                    if let Some(history) = &session.history {
                        if history.idle_time() > config.max_idle_time {
                            stats.idle_removed += 1;
                            true
                        } else if session_age > config.max_session_age {
                            stats.aged_removed += 1;
                            true
                        } else {
                            false
                        }
                    } else if session_age > config.max_session_age {
                        stats.aged_removed += 1;
                        true
                    } else {
                        false
                    }
                }
            };

            if should_remove {
                to_remove.push(id.clone());
            }
        }

        if let Some(max_sessions) = config.max_sessions {
            if total_sessions > max_sessions {
                let excess = total_sessions - max_sessions;
                let mut idle_sessions: Vec<_> = sessions.iter()
                    .filter(|(id, s)| {
                        !to_remove.contains(id) &&
                        !matches!(s.call_state, CallState::Active | CallState::OnHold | CallState::Bridged)
                    })
                    .map(|(id, s)| (id.clone(), s.created_at))
                    .collect();

                idle_sessions.sort_by_key(|(_, created)| *created);

                for (id, _) in idle_sessions.iter().take(excess) {
                    to_remove.push(id.clone());
                    stats.memory_pressure_removed += 1;
                }
            }
        }

        if let Some(max_bytes) = config.max_memory_bytes {
            let estimated_size = total_sessions * std::mem::size_of::<SessionState>();
            if estimated_size > max_bytes {
                let mut idle_sessions: Vec<_> = sessions.iter()
                    .filter(|(id, s)| {
                        !to_remove.contains(id) &&
                        !matches!(s.call_state, CallState::Active | CallState::OnHold | CallState::Bridged)
                    })
                    .map(|(id, s)| (id.clone(), s.created_at))
                    .collect();

                idle_sessions.sort_by_key(|(_, created)| *created);

                for (id, _) in idle_sessions {
                    to_remove.push(id);
                    stats.memory_pressure_removed += 1;

                    let new_size = (total_sessions - to_remove.len()) * std::mem::size_of::<SessionState>();
                    if new_size < max_bytes {
                        break;
                    }
                }
            }
        }

        drop(sessions);

        if !to_remove.is_empty() {
            let index_keys: Vec<(SessionId, Option<DialogId>, Option<CallId>, Option<MediaSessionId>)> = {
                let sessions = self.sessions.read().await;
                to_remove.iter().filter_map(|id| {
                    sessions.get(id).map(|s| (
                        id.clone(),
                        s.dialog_id.clone(),
                        s.call_id.clone(),
                        s.media_session_id.clone(),
                    ))
                }).collect()
            };

            {
                let mut sessions = self.sessions.write().await;
                for id in &to_remove {
                    sessions.remove(id);
                }
            }

            {
                let mut by_dialog = self.by_dialog.write().await;
                let mut by_call_id = self.by_call_id.write().await;
                let mut by_media_id = self.by_media_id.write().await;

                for (_session_id, dialog_id, call_id, media_session_id) in &index_keys {
                    if let Some(did) = dialog_id {
                        by_dialog.remove(did);
                    }
                    if let Some(cid) = call_id {
                        by_call_id.remove(cid);
                    }
                    if let Some(mid) = media_session_id {
                        by_media_id.remove(mid);
                    }
                }
            }

            stats.total_removed = to_remove.len();
        }

        stats.duration = start.elapsed();
        stats
    }

    /// Start automatic cleanup task
    pub fn start_cleanup_task(self: Arc<Self>, config: CleanupConfig) -> tokio::task::JoinHandle<()> {
        if !config.enabled {
            info!("Automatic cleanup is disabled");
            return tokio::spawn(async {});
        }

        info!("Starting automatic cleanup task with interval: {:?}", config.interval);

        return tokio::spawn(async move {
            let mut cleanup_interval = interval(config.interval);
            let mut _consecutive_failures = 0;

            loop {
                cleanup_interval.tick().await;

                let stats = self.cleanup_sessions(config.clone()).await;

                _consecutive_failures = 0;

                if stats.total_removed > 0 {
                    info!(
                        "Cleanup completed: removed {} of {} sessions in {:?}",
                        stats.total_removed,
                        stats.sessions_checked,
                        stats.duration
                    );

                    debug!(
                        "Cleanup details: terminated={}, failed={}, idle={}, aged={}, memory={}",
                        stats.terminated_removed,
                        stats.failed_removed,
                        stats.idle_removed,
                        stats.aged_removed,
                        stats.memory_pressure_removed
                    );
                } else {
                    debug!(
                        "Cleanup completed: no sessions removed (examined {} in {:?})",
                        stats.sessions_checked,
                        stats.duration
                    );
                }
            }
        });
    }
}
