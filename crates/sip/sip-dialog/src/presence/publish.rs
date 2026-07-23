//! Reserved PUBLISH API surface for presence (RFC 3903)
//!
//! A transaction-backed PUBLISH implementation is not currently part of the
//! supported dialog feature set. The public builder and publisher types remain
//! available for API compatibility, but every operation fails closed instead of
//! fabricating a SIP response or entity tag.

use crate::{DialogError, DialogResult};
use rvoip_sip_core::{types::pidf::PidfDocument, Uri};
use std::time::Duration;
use tracing::{debug, info};

const PUBLISH_UNSUPPORTED_MESSAGE: &str =
    "SIP PUBLISH is unsupported because no transaction-backed implementation is installed";

fn publish_unsupported_error() -> DialogError {
    DialogError::ProtocolError {
        message: PUBLISH_UNSUPPORTED_MESSAGE.to_string(),
    }
}

/// PUBLISH request builder for presence updates
pub struct PublishBuilder {
    /// Target presence server URI
    target: Uri,

    /// From URI (presentity)
    from: Uri,

    /// Event package (typically "presence")
    event: String,

    /// Entity-tag for conditional updates
    sip_if_match: Option<String>,

    /// Expiration time in seconds
    expires: u32,

    /// Presence document to publish
    body: Option<PidfDocument>,

    /// Marker retaining the public builder shape while support is disabled.
    _unsupported_marker: std::marker::PhantomData<()>,
}

impl PublishBuilder {
    /// Create a new PUBLISH request builder
    pub fn new(target: Uri, from: Uri) -> Self {
        Self {
            target,
            from,
            event: "presence".to_string(),
            sip_if_match: None,
            expires: 3600, // Default 1 hour
            body: None,
            _unsupported_marker: std::marker::PhantomData,
        }
    }

    /// Set the event package (default: "presence")
    pub fn event(mut self, event: impl Into<String>) -> Self {
        self.event = event.into();
        self
    }

    /// Set the SIP-If-Match header for conditional updates
    pub fn if_match(mut self, etag: impl Into<String>) -> Self {
        self.sip_if_match = Some(etag.into());
        self
    }

    /// Set the expiration time in seconds
    pub fn expires(mut self, seconds: u32) -> Self {
        self.expires = seconds;
        self
    }

    /// Set the presence document to publish
    pub fn body(mut self, pidf: PidfDocument) -> Self {
        self.body = Some(pidf);
        self
    }

    /// Attempt to send the configured PUBLISH request.
    ///
    /// PUBLISH is not currently backed by the transaction layer, so this
    /// method fails closed. It never writes to the wire and never fabricates a
    /// successful response or entity tag.
    pub async fn send(self) -> DialogResult<PublishResponse> {
        // Consume the retained builder configuration without materializing a
        // request. These fields remain solely to preserve the public builder
        // contract until a canonical transaction-owned implementation exists.
        let _configured_request = (
            self.target,
            self.from,
            self.event,
            self.sip_if_match,
            self.expires,
            self.body,
        );
        Err(publish_unsupported_error())
    }
}

/// Response from a PUBLISH request
#[derive(Clone)]
pub struct PublishResponse {
    /// SIP status code
    pub status_code: u16,

    /// Entity-tag for subsequent updates
    pub entity_tag: Option<String>,

    /// Granted expiration time
    pub expires: u32,
}

impl std::fmt::Debug for PublishResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PublishResponse")
            .field("status_code", &self.status_code)
            .field("entity_tag_present", &self.entity_tag.is_some())
            .field("entity_tag_len", &self.entity_tag.as_ref().map(String::len))
            .field("expires", &self.expires)
            .finish()
    }
}

impl PublishResponse {
    /// Check if the PUBLISH was successful
    pub fn is_success(&self) -> bool {
        self.status_code >= 200 && self.status_code < 300
    }
}

/// Presence publisher for managing PUBLISH state
pub struct PresencePublisher {
    /// Target presence server
    target: Uri,

    /// Presentity URI
    presentity: Uri,

    /// Current entity-tag
    entity_tag: Option<String>,

    /// Auto-refresh interval
    refresh_interval: Duration,
}

impl PresencePublisher {
    /// Create a new presence publisher
    pub fn new(target: Uri, presentity: Uri) -> Self {
        Self {
            target,
            presentity,
            entity_tag: None,
            refresh_interval: Duration::from_secs(3300), // 55 minutes
        }
    }

    /// Publish presence information
    pub async fn publish(&mut self, pidf: PidfDocument) -> DialogResult<()> {
        let mut builder = PublishBuilder::new(self.target.clone(), self.presentity.clone());

        // Add entity-tag for updates
        if let Some(etag) = &self.entity_tag {
            builder = builder.if_match(etag);
        }

        let response = builder.body(pidf).send().await?;

        if !response.is_success() {
            return Err(DialogError::ProtocolError {
                message: format!("PUBLISH failed with status {}", response.status_code),
            });
        }

        // Update entity-tag for next update
        if let Some(etag) = response.entity_tag {
            self.entity_tag = Some(etag);
            info!(
                "Presence published, entity_tag_present={}",
                self.entity_tag.is_some()
            );
        }

        Ok(())
    }

    /// Refresh the publication (keep-alive)
    pub async fn refresh(&mut self) -> DialogResult<()> {
        let mut builder = PublishBuilder::new(self.target.clone(), self.presentity.clone());
        if let Some(etag) = &self.entity_tag {
            builder = builder.if_match(etag);
        }
        let response = builder.send().await?;

        if !response.is_success() {
            // Lost our publication, need to re-publish
            self.entity_tag = None;
            return Err(DialogError::ProtocolError {
                message: format!("Refresh failed with status {}", response.status_code),
            });
        }

        debug!("Presence publication refreshed");
        Ok(())
    }

    /// Remove the publication
    pub async fn remove(&mut self) -> DialogResult<()> {
        let mut builder = PublishBuilder::new(self.target.clone(), self.presentity.clone());
        if let Some(etag) = &self.entity_tag {
            builder = builder.if_match(etag);
        }
        let response = builder.expires(0).send().await?;

        if !response.is_success() {
            return Err(DialogError::ProtocolError {
                message: format!("Remove failed with status {}", response.status_code),
            });
        }
        self.entity_tag = None;
        info!("Presence publication removed");
        Ok(())
    }

    /// Get the current entity-tag
    pub fn entity_tag(&self) -> Option<&str> {
        self.entity_tag.as_deref()
    }

    /// Get the refresh interval
    pub fn refresh_interval(&self) -> Duration {
        self.refresh_interval
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_publish_is_unsupported<T>(result: DialogResult<T>) {
        match result {
            Err(DialogError::ProtocolError { message }) => {
                assert_eq!(message, PUBLISH_UNSUPPORTED_MESSAGE);
            }
            Err(error) => panic!(
                "expected explicit unsupported PUBLISH error, got class={}",
                error.diagnostic_class()
            ),
            Ok(_) => panic!("unsupported PUBLISH operation reported success"),
        }
    }

    fn publisher() -> PresencePublisher {
        PresencePublisher::new(
            "sip:presence.example.invalid".parse().unwrap(),
            "sip:alice@example.invalid".parse().unwrap(),
        )
    }

    #[tokio::test]
    async fn publish_builder_fails_closed_without_fabricated_response() {
        let result = PublishBuilder::new(
            "sip:presence.example.invalid".parse().unwrap(),
            "sip:alice@example.invalid".parse().unwrap(),
        )
        .event("presence")
        .expires(300)
        .body(PidfDocument::available("pres:alice@example.invalid"))
        .send()
        .await;

        assert_publish_is_unsupported(result);
    }

    #[tokio::test]
    async fn unsupported_publisher_operations_preserve_entity_tag_state() {
        let mut publisher = publisher();

        let result = publisher
            .publish(PidfDocument::available("pres:alice@example.invalid"))
            .await;
        assert_publish_is_unsupported(result);
        assert_eq!(publisher.entity_tag(), None);

        assert_publish_is_unsupported(publisher.refresh().await);
        assert_eq!(publisher.entity_tag(), None);

        assert_publish_is_unsupported(publisher.remove().await);
        assert_eq!(publisher.entity_tag(), None);

        const EXISTING_TAG: &str = "existing-publish-etag";
        publisher.entity_tag = Some(EXISTING_TAG.to_string());

        let result = publisher
            .publish(PidfDocument::unavailable("pres:alice@example.invalid"))
            .await;
        assert_publish_is_unsupported(result);
        assert_eq!(publisher.entity_tag(), Some(EXISTING_TAG));

        assert_publish_is_unsupported(publisher.refresh().await);
        assert_eq!(publisher.entity_tag(), Some(EXISTING_TAG));

        assert_publish_is_unsupported(publisher.remove().await);
        assert_eq!(publisher.entity_tag(), Some(EXISTING_TAG));
    }

    #[test]
    fn publish_response_debug_hides_entity_tag() {
        const SECRET: &str = "publish-tag-secret-canary";
        let response = PublishResponse {
            status_code: 200,
            entity_tag: Some(SECRET.to_string()),
            expires: 300,
        };
        let debug = format!("{response:?}");

        assert!(!debug.contains(SECRET));
        assert!(debug.contains("entity_tag_present: true"));
    }
}
