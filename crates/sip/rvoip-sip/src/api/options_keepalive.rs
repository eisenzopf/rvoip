//! Periodic out-of-dialog `OPTIONS` pings to configured peers.
//!
//! Driven by [`Config::options_keepalive_targets`] and
//! [`Config::options_keepalive_interval_secs`]. One task per coordinator pings
//! every target concurrently once per interval, starting one second (or one
//! interval, if shorter) after the coordinator starts, and
//! publishes [`Event::PeerReachabilityChanged`] for each target's first
//! outcome and every later change. The task holds only a weak coordinator
//! reference between rounds and stops on coordinator shutdown or drop.
//!
//! [`Config::options_keepalive_targets`]: crate::Config::options_keepalive_targets
//! [`Config::options_keepalive_interval_secs`]: crate::Config::options_keepalive_interval_secs

use std::sync::Weak;
use std::time::Duration;

use rvoip_sip_core::types::address::Address;
use rvoip_sip_core::types::contact::{Contact, ContactParamInfo};
use rvoip_sip_core::types::TypedHeader;
use rvoip_sip_core::Uri;
use tokio::sync::watch;
use tokio::time::MissedTickBehavior;

use crate::api::events::Event;
use crate::api::unified::{Config, UnifiedCoordinator};

/// Longest a single ping waits for its final response.
const MAX_PING_WAIT: Duration = Duration::from_secs(8);

/// Delay before the first round, so an application that subscribes to
/// events right after constructing its peer still sees the first outcome.
const FIRST_ROUND_DELAY: Duration = Duration::from_secs(1);

/// Extra time allowed past the transaction timeout before the ping is
/// abandoned locally, so a stalled send path cannot wedge the loop.
const PING_ABANDON_GRACE: Duration = Duration::from_secs(1);

/// What one ping established about its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PingOutcome {
    pub(crate) reachable: bool,
    pub(crate) status_code: Option<u16>,
}

impl PingOutcome {
    /// A final response proves the peer's SIP stack is answering, except
    /// `408` (a proxy or the stack itself timed out) and `503` (the peer
    /// says it is unavailable).
    pub(crate) fn from_status(status_code: u16) -> Self {
        Self {
            reachable: !matches!(status_code, 408 | 503),
            status_code: Some(status_code),
        }
    }

    pub(crate) fn no_response() -> Self {
        Self {
            reachable: false,
            status_code: None,
        }
    }
}

/// Immutable inputs for the keep-alive loop, captured from [`Config`].
pub(crate) struct KeepaliveSettings {
    targets: Vec<String>,
    interval: Duration,
    ping_wait: Duration,
    from_uri: String,
    contact: Option<TypedHeader>,
}

impl KeepaliveSettings {
    /// `None` when the configuration does not ask for pings.
    pub(crate) fn from_config(config: &Config) -> Option<Self> {
        if config.options_keepalive_interval_secs == 0
            || config.options_keepalive_targets.is_empty()
        {
            return None;
        }
        let interval = Duration::from_secs(config.options_keepalive_interval_secs);
        let contact = config
            .contact_uri
            .as_deref()
            .and_then(|uri| uri.parse::<Uri>().ok())
            .map(|uri| {
                TypedHeader::Contact(Contact::new_params(vec![ContactParamInfo {
                    address: Address::new(uri),
                }]))
            });
        Some(Self {
            targets: config.options_keepalive_targets.clone(),
            interval,
            ping_wait: interval.min(MAX_PING_WAIT),
            from_uri: config.local_uri.clone(),
            contact,
        })
    }
}

/// Last published reachability per target; decides what to publish next.
#[derive(Debug)]
pub(crate) struct ReachabilityTracker {
    last: Vec<Option<bool>>,
}

impl ReachabilityTracker {
    pub(crate) fn new(targets: usize) -> Self {
        Self {
            last: vec![None; targets],
        }
    }

    /// Record `outcome` for target `index`; `true` when it is the first
    /// outcome or differs from the last one, i.e. when an event is due.
    pub(crate) fn observe(&mut self, index: usize, outcome: PingOutcome) -> bool {
        let Some(slot) = self.last.get_mut(index) else {
            return false;
        };
        let changed = *slot != Some(outcome.reachable);
        *slot = Some(outcome.reachable);
        changed
    }
}

async fn ping(
    coordinator: &UnifiedCoordinator,
    settings: &KeepaliveSettings,
    target: &str,
) -> PingOutcome {
    let opts = rvoip_sip_dialog::api::unified::OptionsRequestOptions {
        from_uri: settings.from_uri.clone(),
        to_uri: target.to_string(),
        accept: None,
        timeout: Some(settings.ping_wait),
        cseq: None,
        call_id: None,
        from_tag: None,
        extra_headers: settings.contact.iter().cloned().collect(),
    };
    match tokio::time::timeout(
        settings.ping_wait + PING_ABANDON_GRACE,
        coordinator.send_options_oob_with_optional_auth(opts, None),
    )
    .await
    {
        Ok(Ok(response)) => PingOutcome::from_status(response.status_code()),
        Ok(Err(_)) | Err(_) => PingOutcome::no_response(),
    }
}

/// Ping every target once per interval until shutdown or coordinator drop.
pub(crate) async fn run(
    coordinator: Weak<UnifiedCoordinator>,
    settings: KeepaliveSettings,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut tracker = ReachabilityTracker::new(settings.targets.len());
    let first_round = tokio::time::Instant::now() + settings.interval.min(FIRST_ROUND_DELAY);
    let mut ticker = tokio::time::interval_at(first_round, settings.interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        if *shutdown.borrow() {
            return;
        }
        tokio::select! {
            _ = ticker.tick() => {}
            changed = shutdown.changed() => {
                if changed.is_err() {
                    return;
                }
                continue;
            }
        }
        let Some(strong) = coordinator.upgrade() else {
            return;
        };
        let round = futures::future::join_all(
            settings
                .targets
                .iter()
                .map(|target| ping(&strong, &settings, target)),
        );
        let outcomes = tokio::select! {
            outcomes = round => outcomes,
            _ = shutdown.changed() => return,
        };
        for (index, outcome) in outcomes.into_iter().enumerate() {
            if !tracker.observe(index, outcome) {
                continue;
            }
            let target = settings.targets[index].clone();
            if outcome.reachable {
                tracing::info!(status = ?outcome.status_code, "OPTIONS keep-alive peer reachable");
            } else {
                tracing::warn!(status = ?outcome.status_code, "OPTIONS keep-alive peer unreachable");
            }
            strong.publish_app_event(Event::PeerReachabilityChanged {
                target,
                reachable: outcome.reachable,
                status_code: outcome.status_code,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_final_response_but_408_and_503_is_reachable() {
        for code in [200, 401, 403, 404, 405, 486, 500, 502, 600] {
            assert!(PingOutcome::from_status(code).reachable, "{code}");
        }
        for code in [408, 503] {
            assert!(!PingOutcome::from_status(code).reachable, "{code}");
        }
        assert_eq!(PingOutcome::no_response().status_code, None);
        assert!(!PingOutcome::no_response().reachable);
    }

    #[test]
    fn tracker_reports_first_outcome_and_changes_only() {
        let mut tracker = ReachabilityTracker::new(2);
        let up = PingOutcome::from_status(200);
        let down = PingOutcome::no_response();
        assert!(tracker.observe(0, up), "first outcome is reported");
        assert!(
            !tracker.observe(0, PingOutcome::from_status(404)),
            "still up"
        );
        assert!(tracker.observe(0, down), "up -> down");
        assert!(!tracker.observe(0, down), "still down");
        assert!(tracker.observe(0, up), "down -> up");
        assert!(tracker.observe(1, down), "targets are independent");
        assert!(!tracker.observe(2, up), "unknown index is ignored");
    }

    #[test]
    fn settings_require_targets_and_a_nonzero_interval() {
        let mut config = Config::local("sbc", 5060);
        assert!(KeepaliveSettings::from_config(&config).is_none());
        config
            .options_keepalive_targets
            .push("sip:peer.example.com".to_string());
        config.options_keepalive_interval_secs = 0;
        assert!(KeepaliveSettings::from_config(&config).is_none());
        config.options_keepalive_interval_secs = 60;
        config.contact_uri = Some("sip:sbc.example.com:5061;transport=tls".to_string());
        let settings = KeepaliveSettings::from_config(&config).expect("enabled");
        assert_eq!(settings.ping_wait, MAX_PING_WAIT);
        assert!(matches!(settings.contact, Some(TypedHeader::Contact(_))));
        config.options_keepalive_interval_secs = 2;
        let settings = KeepaliveSettings::from_config(&config).expect("enabled");
        assert_eq!(settings.ping_wait, Duration::from_secs(2));
    }
}
