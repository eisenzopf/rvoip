//! Race-resistant per-session lifecycle observations.
//!
//! This module backs the public handle-first wait APIs. App-visible events are
//! still delivered through the global session event bus; the lifecycle index is
//! an internal companion cache updated immediately before event publication so
//! late waiters can observe recently published lifecycle facts without polling.

use crate::adapters::SessionApiCrossCrateEvent;
use crate::api::events::{Event, MediaSecurityState};
use crate::api::handle::TransferOutcome;
use crate::errors::{Result, SessionError};
use crate::state_table::types::SessionId;
use crate::types::CallState;
use dashmap::DashMap;
use rvoip_infra_common::events::coordinator::GlobalEventCoordinator;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

const TERMINAL_EVENT_TTL: Duration = Duration::from_secs(60);
const MAX_PROGRESS_EVENTS: usize = 8;

/// Provisional call-progress evidence observed for a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallProgressInfo {
    /// Session identifier for the call.
    pub call_id: SessionId,
    /// SIP provisional status code, usually `180` or `183`.
    pub status_code: u16,
    /// SIP reason phrase.
    pub reason: String,
    /// SDP body carried by the provisional response, if present.
    pub sdp: Option<String>,
}

impl CallProgressInfo {
    fn from_event(event: &Event) -> Option<Self> {
        match event {
            Event::CallProgress {
                call_id,
                status_code,
                reason,
                sdp,
            } => Some(Self {
                call_id: call_id.clone(),
                status_code: *status_code,
                reason: reason.clone(),
                sdp: sdp.clone(),
            }),
            _ => None,
        }
    }

    pub(crate) fn to_event(&self) -> Event {
        Event::CallProgress {
            call_id: self.call_id.clone(),
            status_code: self.status_code,
            reason: self.reason.clone(),
            sdp: self.sdp.clone(),
        }
    }
}

/// Answer evidence observed for a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallAnsweredInfo {
    /// Session identifier for the answered call.
    pub call_id: SessionId,
    /// SDP body from the answer, if present.
    pub sdp: Option<String>,
}

impl CallAnsweredInfo {
    fn from_event(event: &Event) -> Option<Self> {
        match event {
            Event::CallAnswered { call_id, sdp } => Some(Self {
                call_id: call_id.clone(),
                sdp: sdp.clone(),
            }),
            _ => None,
        }
    }
}

/// Terminal lifecycle evidence for a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallTerminalInfo {
    /// Normal call end.
    Ended {
        /// Human-readable teardown reason.
        reason: String,
    },
    /// Call setup or dialog failed.
    Failed {
        /// SIP status code or synthesized failure code.
        status_code: u16,
        /// Human-readable failure reason.
        reason: String,
    },
    /// Caller cancelled the call before answer.
    Cancelled,
}

impl CallTerminalInfo {
    fn from_event(event: &Event) -> Option<(SessionId, Self)> {
        match event {
            Event::CallEnded { call_id, reason } => Some((
                call_id.clone(),
                Self::Ended {
                    reason: reason.clone(),
                },
            )),
            Event::CallFailed {
                call_id,
                status_code,
                reason,
            } => Some((
                call_id.clone(),
                Self::Failed {
                    status_code: *status_code,
                    reason: reason.clone(),
                },
            )),
            Event::CallCancelled { call_id } => Some((call_id.clone(), Self::Cancelled)),
            _ => None,
        }
    }

    pub(crate) fn reason(&self) -> String {
        match self {
            Self::Ended { reason } => reason.clone(),
            Self::Failed {
                status_code,
                reason,
            } => format!("{status_code}: {reason}"),
            Self::Cancelled => "Cancelled".to_string(),
        }
    }
}

/// Current typed lifecycle view for one call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallLifecycleSnapshot {
    /// Session identifier for this snapshot.
    pub call_id: SessionId,
    /// Current call state from the session store, if still present.
    pub state: Option<CallState>,
    /// Recent provisional progress events, oldest first.
    pub progress: Vec<CallProgressInfo>,
    /// Answer evidence, if the call has answered.
    pub answered: Option<CallAnsweredInfo>,
    /// Negotiated media-security state, if SRTP was negotiated.
    pub media_security: Option<MediaSecurityState>,
    /// Terminal evidence, retained briefly after session cleanup.
    pub terminal: Option<CallTerminalInfo>,
    /// Latest typed transfer outcome observed for this call, if any.
    pub latest_transfer_outcome: Option<TransferOutcome>,
}

#[derive(Debug, Clone)]
struct LifecycleEntry {
    progress: VecDeque<CallProgressInfo>,
    answered: Option<CallAnsweredInfo>,
    media_security: Option<MediaSecurityState>,
    terminal: Option<(CallTerminalInfo, Instant)>,
    latest_transfer_outcome: Option<TransferOutcome>,
}

impl Default for LifecycleEntry {
    fn default() -> Self {
        Self {
            progress: VecDeque::with_capacity(MAX_PROGRESS_EVENTS),
            answered: None,
            media_security: None,
            terminal: None,
            latest_transfer_outcome: None,
        }
    }
}

/// Internal lifecycle index keyed by session id.
#[derive(Debug, Clone, Default)]
pub(crate) struct LifecycleIndex {
    entries: Arc<DashMap<SessionId, LifecycleEntry>>,
}

impl LifecycleIndex {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn record_event(&self, event: &Event) {
        let Some(call_id) = event.call_id().cloned() else {
            return;
        };

        let mut entry = self.entries.entry(call_id.clone()).or_default();

        if let Some(progress) = CallProgressInfo::from_event(event) {
            if entry.progress.len() == MAX_PROGRESS_EVENTS {
                entry.progress.pop_front();
            }
            entry.progress.push_back(progress);
        }

        if let Some(answered) = CallAnsweredInfo::from_event(event) {
            entry.answered = Some(answered);
        }

        if let Event::MediaSecurityNegotiated {
            keying,
            suite,
            profile,
            contexts_installed,
            ..
        } = event
        {
            entry.media_security = Some(MediaSecurityState {
                keying: *keying,
                suite: *suite,
                profile: *profile,
                contexts_installed: *contexts_installed,
            });
        }

        if let Ok(outcome) = TransferOutcome::try_from(event.clone()) {
            entry.latest_transfer_outcome = Some(outcome);
        }

        if let Some((_, terminal)) = CallTerminalInfo::from_event(event) {
            entry.terminal = Some((terminal, Instant::now()));
        }
    }

    pub(crate) fn snapshot(
        &self,
        call_id: &SessionId,
        state: Option<CallState>,
    ) -> CallLifecycleSnapshot {
        let mut terminal_expired = false;
        let snapshot = if let Some(entry) = self.entries.get(call_id) {
            let terminal = entry.terminal.as_ref().and_then(|(terminal, stored_at)| {
                if stored_at.elapsed() <= TERMINAL_EVENT_TTL {
                    Some(terminal.clone())
                } else {
                    terminal_expired = true;
                    None
                }
            });
            CallLifecycleSnapshot {
                call_id: call_id.clone(),
                state,
                progress: entry.progress.iter().cloned().collect(),
                answered: entry.answered.clone(),
                media_security: entry.media_security.clone(),
                terminal,
                latest_transfer_outcome: entry.latest_transfer_outcome.clone(),
            }
        } else {
            CallLifecycleSnapshot {
                call_id: call_id.clone(),
                state,
                progress: Vec::new(),
                answered: None,
                media_security: None,
                terminal: None,
                latest_transfer_outcome: None,
            }
        };

        if terminal_expired {
            self.entries.remove(call_id);
        }

        snapshot
    }
}

/// Publishes app-level session events and updates lifecycle first.
#[derive(Clone)]
pub(crate) struct SessionEventPublisher {
    coordinator: Arc<GlobalEventCoordinator>,
    lifecycle: LifecycleIndex,
}

impl SessionEventPublisher {
    pub(crate) fn new(coordinator: Arc<GlobalEventCoordinator>, lifecycle: LifecycleIndex) -> Self {
        Self {
            coordinator,
            lifecycle,
        }
    }

    pub(crate) fn publish(&self, event: Event) {
        self.lifecycle.record_event(&event);
        let wrapped = SessionApiCrossCrateEvent::new(event);
        let coordinator = self.coordinator.clone();
        tokio::spawn(async move {
            if let Err(e) = coordinator.publish(wrapped).await {
                tracing::warn!("Failed to publish app-level event: {}", e);
            }
        });
    }

    pub(crate) async fn publish_now(&self, event: Event) -> Result<()> {
        self.lifecycle.record_event(&event);
        let wrapped = SessionApiCrossCrateEvent::new(event);
        self.coordinator
            .publish(wrapped)
            .await
            .map_err(|e| SessionError::Other(format!("Failed to publish app-level event: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_records_progress_and_terminal() {
        let index = LifecycleIndex::new();
        let call_id = SessionId::new();

        index.record_event(&Event::CallProgress {
            call_id: call_id.clone(),
            status_code: 183,
            reason: "Session Progress".to_string(),
            sdp: Some("v=0".to_string()),
        });
        index.record_event(&Event::CallCancelled {
            call_id: call_id.clone(),
        });

        let snapshot = index.snapshot(&call_id, Some(CallState::Cancelled));
        assert_eq!(snapshot.progress.len(), 1);
        assert_eq!(snapshot.progress[0].status_code, 183);
        assert_eq!(snapshot.terminal, Some(CallTerminalInfo::Cancelled));
        assert_eq!(
            snapshot.terminal.as_ref().map(CallTerminalInfo::reason),
            Some("Cancelled".to_string())
        );
    }

    #[test]
    fn lifecycle_records_media_security() {
        let index = LifecycleIndex::new();
        let call_id = SessionId::new();

        index.record_event(&Event::MediaSecurityNegotiated {
            call_id: call_id.clone(),
            keying: crate::api::events::MediaSecurityKeying::Sdes,
            suite: rvoip_sip_core::types::sdp::CryptoSuite::AesCm128HmacSha1_80,
            profile: crate::api::events::MediaSecurityProfile::RtpSavp,
            contexts_installed: true,
        });

        let snapshot = index.snapshot(&call_id, None);
        assert!(snapshot.media_security.is_some());
    }
}
