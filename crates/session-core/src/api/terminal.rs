//! Short-lived terminal event cache for deterministic late waits.

use crate::api::events::Event;
use crate::state_table::types::SessionId;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(crate) const TERMINAL_EVENT_TTL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TerminalEvent {
    Ended(String),
    Failed { status_code: u16, reason: String },
    Cancelled,
}

impl TerminalEvent {
    pub(crate) fn from_event(event: &Event) -> Option<(SessionId, Self)> {
        match event {
            Event::CallEnded { call_id, reason } => {
                Some((call_id.clone(), Self::Ended(reason.clone())))
            }
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
            Self::Ended(reason) => reason.clone(),
            Self::Failed {
                status_code,
                reason,
            } => format!("{status_code}: {reason}"),
            Self::Cancelled => "Cancelled".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TerminalCacheEntry {
    pub(crate) event: TerminalEvent,
    pub(crate) stored_at: Instant,
}

pub(crate) type TerminalEventCache = Arc<DashMap<SessionId, TerminalCacheEntry>>;

pub(crate) fn remember_terminal_event(cache: &TerminalEventCache, event: &Event) {
    if let Some((call_id, terminal)) = TerminalEvent::from_event(event) {
        cache.insert(
            call_id,
            TerminalCacheEntry {
                event: terminal,
                stored_at: Instant::now(),
            },
        );
    }
}

pub(crate) fn cached_terminal_event(
    cache: &TerminalEventCache,
    call_id: &SessionId,
) -> Option<TerminalEvent> {
    let entry = cache.get(call_id)?;
    if entry.stored_at.elapsed() <= TERMINAL_EVENT_TTL {
        Some(entry.event.clone())
    } else {
        drop(entry);
        cache.remove(call_id);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache() -> TerminalEventCache {
        Arc::new(DashMap::new())
    }

    #[test]
    fn remembers_terminal_call_ended_event() {
        let cache = cache();
        let call_id = SessionId::new();
        remember_terminal_event(
            &cache,
            &Event::CallEnded {
                call_id: call_id.clone(),
                reason: "Normal".to_string(),
            },
        );

        assert_eq!(
            cached_terminal_event(&cache, &call_id),
            Some(TerminalEvent::Ended("Normal".to_string()))
        );
    }

    #[test]
    fn evicts_expired_terminal_events() {
        let cache = cache();
        let call_id = SessionId::new();
        cache.insert(
            call_id.clone(),
            TerminalCacheEntry {
                event: TerminalEvent::Cancelled,
                stored_at: Instant::now() - TERMINAL_EVENT_TTL - Duration::from_secs(1),
            },
        );

        assert_eq!(cached_terminal_event(&cache, &call_id), None);
        assert!(!cache.contains_key(&call_id));
    }
}
