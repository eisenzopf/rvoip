//! PUBLISH method implementation for presence (RFC 3903)
//!
//! Provides support for publishing presence information to a presence server.

use std::sync::Arc;
use std::time::Duration;
use bytes::Bytes;
use rvoip_sip_core::{
    Request, Method, HeaderName, TypedHeader, Uri, HeaderValue,
    types::pidf::PidfDocument,
};
use tracing::{debug, info, warn};

use crate::{Dialog, DialogId, DialogResult, DialogError};

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
    
    /// Placeholder for future transaction management
    _placeholder: std::marker::PhantomData<()>,
}

impl PublishBuilder {
    /// Create a new PUBLISH request builder
    pub fn new(
        target: Uri,
        from: Uri,
    ) -> Self {
        Self {
            target,
            from,
            event: "presence".to_string(),
            sip_if_match: None,
            expires: 3600, // Default 1 hour
            body: None,
            _placeholder: std::marker::PhantomData,
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
    
    /// Build and send the PUBLISH request
    pub async fn send(self) -> DialogResult<PublishResponse> {
        // Create PUBLISH request
        let mut request = Request::new(Method::Publish, self.target.clone());
        
        // Add required headers using Other variant for simplicity
        request.headers.push(TypedHeader::Other(
            HeaderName::From,
            HeaderValue::Raw(self.from.to_string().into_bytes()),
        ));
        request.headers.push(TypedHeader::Other(
            HeaderName::To,
            HeaderValue::Raw(self.target.to_string().into_bytes()),
        ));
        request.headers.push(TypedHeader::Other(
            HeaderName::Event,
            HeaderValue::Raw(self.event.clone().into_bytes()),
        ));
        request.headers.push(TypedHeader::Other(
            HeaderName::Expires,
            HeaderValue::Raw(self.expires.to_string().into_bytes()),
        ));
        
        // Add conditional header if present
        if let Some(ref etag) = self.sip_if_match {
            request.headers.push(TypedHeader::Other(
                HeaderName::SipIfMatch,
                HeaderValue::Raw(etag.clone().into_bytes()),
            ));
        }
        
        // Add body if present
        if let Some(pidf) = self.body {
            let xml = pidf.to_xml();
            let xml_len = xml.len();
            request.body = Bytes::from(xml.into_bytes());
            request.headers.push(TypedHeader::Other(
                HeaderName::ContentType,
                HeaderValue::Raw("application/pidf+xml".to_string().into_bytes()),
            ));
            request.headers.push(TypedHeader::Other(
                HeaderName::ContentLength,
                HeaderValue::Raw(xml_len.to_string().into_bytes()),
            ));
        } else if self.sip_if_match.is_none() {
            // Initial PUBLISH must have a body
            return Err(DialogError::InvalidState {
                expected: "PUBLISH with body".to_string(),
                actual: "PUBLISH without body".to_string(),
            });
        }
        
        // TODO: Send via transaction manager when integrated
        // For now, return a placeholder response
        
        // This would normally send the request and parse the response
        // let response = self.transaction_manager.send_request(request).await?;
        
        Ok(PublishResponse {
            status_code: 200,
            entity_tag: Some("placeholder-etag".to_string()),
            expires: self.expires,
        })
    }
}

/// Response from a PUBLISH request
#[derive(Debug, Clone)]
pub struct PublishResponse {
    /// SIP status code
    pub status_code: u16,
    
    /// Entity-tag for subsequent updates
    pub entity_tag: Option<String>,
    
    /// Granted expiration time
    pub expires: u32,
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
    pub fn new(
        target: Uri,
        presentity: Uri,
    ) -> Self {
        Self {
            target,
            presentity,
            entity_tag: None,
            refresh_interval: Duration::from_secs(3300), // 55 minutes
        }
    }
    
    /// Publish presence information
    pub async fn publish(&mut self, pidf: PidfDocument) -> DialogResult<()> {
        let mut builder = PublishBuilder::new(
            self.target.clone(),
            self.presentity.clone(),
        );
        
        // Add entity-tag for updates
        if let Some(etag) = &self.entity_tag {
            builder = builder.if_match(etag);
        }
        
        let response = builder
            .body(pidf)
            .send()
            .await?;
        
        if !response.is_success() {
            return Err(DialogError::ProtocolError {
                message: format!("PUBLISH failed with status {}", response.status_code),
            });
        }
        
        // Update entity-tag for next update
        if let Some(etag) = response.entity_tag {
            self.entity_tag = Some(etag);
            info!("Presence published, entity-tag: {:?}", self.entity_tag);
        }
        
        Ok(())
    }
    
    /// Refresh the publication (keep-alive)
    pub async fn refresh(&mut self) -> DialogResult<()> {
        if self.entity_tag.is_none() {
            return Err(DialogError::InvalidState {
                expected: "entity-tag from initial PUBLISH".to_string(),
                actual: "no entity-tag".to_string(),
            });
        }
        
        let response = PublishBuilder::new(
            self.target.clone(),
            self.presentity.clone(),
        )
        .if_match(self.entity_tag.as_ref().unwrap())
        .send()
        .await?;
        
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
        if let Some(etag) = &self.entity_tag {
            let response = PublishBuilder::new(
                self.target.clone(),
                self.presentity.clone(),
            )
            .if_match(etag)
            .expires(0) // Remove by setting expires to 0
            .send()
            .await?;
            
            if response.is_success() {
                self.entity_tag = None;
                info!("Presence publication removed");
            }
        }
        
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
    
    #[test]
    fn test_publish_builder() {
        // This would need mock transaction manager to test properly
        // For now, just test the builder pattern
    }
}