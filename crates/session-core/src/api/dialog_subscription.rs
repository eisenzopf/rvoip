//! Public handle for RFC 4235 dialog-package subscriptions.

use crate::api::dialog_package::DialogInfo;
use crate::api::events::Event;
use crate::api::stream_peer::EventReceiver;
use crate::api::unified::UnifiedCoordinator;
use crate::errors::{Result, SessionError};
use crate::state_table::types::SessionId;
use std::sync::Arc;
use std::time::Duration;

/// Handle for an RFC 4235 `Event: dialog` subscription.
#[derive(Clone)]
pub struct DialogSubscriptionHandle {
    subscription_id: SessionId,
    target_uri: String,
    coordinator: Arc<UnifiedCoordinator>,
}

impl DialogSubscriptionHandle {
    pub(crate) fn new(
        subscription_id: SessionId,
        target_uri: String,
        coordinator: Arc<UnifiedCoordinator>,
    ) -> Self {
        Self {
            subscription_id,
            target_uri,
            coordinator,
        }
    }

    /// Synthetic session id used to correlate NOTIFY events for this subscription.
    pub fn id(&self) -> &SessionId {
        &self.subscription_id
    }

    /// Subscription target URI.
    pub fn target_uri(&self) -> &str {
        &self.target_uri
    }

    /// Subscribe to events for this dialog subscription.
    pub async fn events(&self) -> Result<EventReceiver> {
        self.coordinator
            .events_for_session(&self.subscription_id)
            .await
    }

    /// Wait for a parsed RFC 4235 dialog entry matching `predicate`.
    pub async fn wait_for_dialog<F>(
        &self,
        mut predicate: F,
        timeout: Option<Duration>,
    ) -> Result<DialogInfo>
    where
        F: FnMut(&DialogInfo) -> bool,
    {
        let mut events = self.events().await?;
        let fut = async {
            loop {
                match events.next().await {
                    Some(Event::DialogPackageNotify { dialogs, .. }) => {
                        if let Some(dialog) = dialogs.into_iter().find(|d| predicate(d)) {
                            return Ok(dialog);
                        }
                    }
                    Some(Event::DialogStateChanged { dialog, .. }) if predicate(&dialog) => {
                        return Ok(dialog);
                    }
                    Some(_) => {}
                    None => {
                        return Err(SessionError::Other(
                            "Event channel closed while waiting for dialog package event"
                                .to_string(),
                        ))
                    }
                }
            }
        };

        match timeout {
            Some(duration) => tokio::time::timeout(duration, fut)
                .await
                .map_err(|_| SessionError::Timeout("wait_for_dialog timed out".to_string()))?,
            None => fut.await,
        }
    }

    /// Terminate the subscription by sending an in-dialog SUBSCRIBE with `Expires: 0`.
    pub async fn unsubscribe(&self) -> Result<()> {
        self.coordinator
            .unsubscribe_dialogs(&self.subscription_id)
            .await
    }
}
