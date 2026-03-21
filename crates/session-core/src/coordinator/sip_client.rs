//! SipClient trait implementation for SessionCoordinator

use std::sync::Arc;
use std::time::Duration;
use std::collections::HashMap;
use async_trait::async_trait;
use rvoip_sip_core::Method;
use rvoip_sip_core::builder::UserAgentBuilderExt;
use crate::api::client::{SipClient, RegistrationHandle, SipResponse, SubscriptionHandle};
use crate::errors::{Result, SessionError};
use super::SessionCoordinator;
use super::registration::{self, RegistrationConfig as RegConfig};

#[async_trait]
impl SipClient for Arc<SessionCoordinator> {
    async fn register(
        &self,
        registrar_uri: &str,
        from_uri: &str,
        contact_uri: &str,
        expires: u32,
    ) -> Result<RegistrationHandle> {
        // Check if SIP client is enabled
        if !self.config.enable_sip_client {
            return Err(SessionError::NotSupported {
                feature: "SIP client operations".to_string(),
                reason: "enable_sip_client must be set to true in configuration".to_string(),
            });
        }

        // Delegate to the registration module which handles auth challenges
        let config = RegConfig {
            registrar_uri: registrar_uri.to_string(),
            from_uri: from_uri.to_string(),
            contact_uri: contact_uri.to_string(),
            expires,
            username: None,
            password: None,
        };

        registration::send_register_with_auth(self, &config, expires).await
    }

    async fn register_with_credentials(
        &self,
        registrar_uri: &str,
        from_uri: &str,
        contact_uri: &str,
        expires: u32,
        username: &str,
        password: &str,
    ) -> Result<RegistrationHandle> {
        if !self.config.enable_sip_client {
            return Err(SessionError::NotSupported {
                feature: "SIP client operations".to_string(),
                reason: "enable_sip_client must be set to true in configuration".to_string(),
            });
        }

        let config = RegConfig {
            registrar_uri: registrar_uri.to_string(),
            from_uri: from_uri.to_string(),
            contact_uri: contact_uri.to_string(),
            expires,
            username: Some(username.to_string()),
            password: Some(password.to_string()),
        };

        registration::send_register_with_auth(self, &config, expires).await
    }

    async fn send_options(&self, target_uri: &str) -> Result<SipResponse> {
        if !self.config.enable_sip_client {
            return Err(SessionError::NotSupported {
                feature: "SIP client operations".to_string(),
                reason: "enable_sip_client must be set to true in configuration".to_string(),
            });
        }
        
        use rvoip_sip_core::builder::SimpleRequestBuilder;
        
        // Generate unique identifiers
        let call_id = format!("opt-{}-{}", std::process::id(), uuid::Uuid::new_v4());
        let from_tag = format!("tag-{}", uuid::Uuid::new_v4().simple());
        let branch = format!("z9hG4bK{}", uuid::Uuid::new_v4().simple());
        
        // Get local address and from URI
        let local_addr = self.get_bound_address();
        let from_uri = &self.config.local_address;
        
        // Build OPTIONS request
        let request = SimpleRequestBuilder::options(target_uri)
            .map_err(|e| SessionError::invalid_uri(&format!("Invalid target URI: {}", e)))?
            .from("", from_uri, Some(&from_tag))
            .to("", target_uri, None)
            .call_id(&call_id)
            .cseq(1)
            .via(&local_addr.to_string(), "UDP", Some(&branch))
            .max_forwards(70)
            .user_agent("RVoIP-SessionCore/1.0")
            .build();
        
        // Parse target URI to get destination address
        let uri: rvoip_sip_core::Uri = target_uri.parse()
            .map_err(|e| SessionError::invalid_uri(&format!("Invalid target URI: {}", e)))?;
        
        // Resolve URI to socket address
        let destination = rvoip_dialog_core::dialog::dialog_utils::uri_resolver::resolve_uri_to_socketaddr(&uri)
            .await
            .ok_or_else(|| SessionError::network_error(&format!("Failed to resolve target address: {}", target_uri)))?;
        
        tracing::info!("Sending OPTIONS to {} ({})", target_uri, destination);
        
        // Send the OPTIONS request via dialog-core
        let response = self.dialog_coordinator.dialog_api()
            .send_non_dialog_request(request, destination, Duration::from_secs(5))
            .await
            .map_err(|e| SessionError::internal(&format!("OPTIONS failed: {}", e)))?;
        
        // Convert to SipResponse
        Ok(SipResponse {
            status_code: response.status_code(),
            reason_phrase: response.reason_phrase().to_string(),
            headers: HashMap::new(), // TODO: Extract headers if needed
            body: if response.body().is_empty() {
                None
            } else {
                Some(String::from_utf8_lossy(response.body()).to_string())
            },
        })
    }
    
    async fn send_message(
        &self,
        to_uri: &str,
        message: &str,
        content_type: Option<&str>,
    ) -> Result<SipResponse> {
        if !self.config.enable_sip_client {
            return Err(SessionError::NotSupported {
                feature: "SIP client operations".to_string(),
                reason: "enable_sip_client must be set to true in configuration".to_string(),
            });
        }
        
        use rvoip_sip_core::builder::SimpleRequestBuilder;
        use rvoip_sip_core::types::{TypedHeader, content_type::ContentType};
        
        // Generate unique identifiers
        let call_id = format!("msg-{}-{}", std::process::id(), uuid::Uuid::new_v4());
        let from_tag = format!("tag-{}", uuid::Uuid::new_v4().simple());
        let branch = format!("z9hG4bK{}", uuid::Uuid::new_v4().simple());
        
        // Get local address and from URI
        let local_addr = self.get_bound_address();
        let from_uri = &self.config.local_address;
        
        // Build MESSAGE request
        let mut builder = SimpleRequestBuilder::new(Method::Message, to_uri)
            .map_err(|e| SessionError::invalid_uri(&format!("Invalid target URI: {}", e)))?
            .from("", from_uri, Some(&from_tag))
            .to("", to_uri, None)
            .call_id(&call_id)
            .cseq(1)
            .via(&local_addr.to_string(), "UDP", Some(&branch))
            .max_forwards(70)
            .user_agent("RVoIP-SessionCore/1.0");
        
        // Add content type header
        let ct = content_type.unwrap_or("text/plain");
        builder = builder.content_type(ct);
        
        // Build request with body
        let request = builder.body(message.to_string()).build();
        
        // Parse target URI to get destination address
        let uri: rvoip_sip_core::Uri = to_uri.parse()
            .map_err(|e| SessionError::invalid_uri(&format!("Invalid target URI: {}", e)))?;
        
        // Resolve URI to socket address
        let destination = rvoip_dialog_core::dialog::dialog_utils::uri_resolver::resolve_uri_to_socketaddr(&uri)
            .await
            .ok_or_else(|| SessionError::network_error(&format!("Failed to resolve target address: {}", to_uri)))?;
        
        tracing::info!("Sending MESSAGE to {} ({})", to_uri, destination);
        
        // Send the MESSAGE request via dialog-core
        let response = self.dialog_coordinator.dialog_api()
            .send_non_dialog_request(request, destination, Duration::from_secs(32))
            .await
            .map_err(|e| SessionError::internal(&format!("MESSAGE failed: {}", e)))?;
        
        // Convert to SipResponse
        Ok(SipResponse {
            status_code: response.status_code(),
            reason_phrase: response.reason_phrase().to_string(),
            headers: HashMap::new(), // TODO: Extract headers if needed
            body: if response.body().is_empty() {
                None
            } else {
                Some(String::from_utf8_lossy(response.body()).to_string())
            },
        })
    }
    
    async fn subscribe(
        &self,
        target_uri: &str,
        event_type: &str,
        expires: u32,
    ) -> Result<SubscriptionHandle> {
        if !self.config.enable_sip_client {
            return Err(SessionError::NotSupported {
                feature: "SIP client operations".to_string(),
                reason: "enable_sip_client must be set to true in configuration".to_string(),
            });
        }

        use rvoip_sip_core::builder::SimpleRequestBuilder;

        // Generate unique identifiers
        let call_id = format!("sub-{}-{}", std::process::id(), uuid::Uuid::new_v4());
        let from_tag = format!("tag-{}", uuid::Uuid::new_v4().simple());
        let branch = format!("z9hG4bK{}", uuid::Uuid::new_v4().simple());

        // Get local address and from URI
        let local_addr = self.get_bound_address();
        let from_uri = &self.config.local_address;

        // Build SUBSCRIBE request (RFC 6665)
        let request = SimpleRequestBuilder::new(Method::Subscribe, target_uri)
            .map_err(|e| SessionError::invalid_uri(&format!("Invalid target URI: {}", e)))?
            .from("", from_uri, Some(&from_tag))
            .to("", target_uri, None)
            .call_id(&call_id)
            .cseq(1)
            .via(&local_addr.to_string(), "UDP", Some(&branch))
            .max_forwards(70)
            .user_agent("RVoIP-SessionCore/1.0")
            .event(event_type)
            .expires(expires)
            .build();

        // Parse target URI to get destination address
        let uri: rvoip_sip_core::Uri = target_uri.parse()
            .map_err(|e| SessionError::invalid_uri(&format!("Invalid target URI: {}", e)))?;

        // Resolve URI to socket address
        let destination = rvoip_dialog_core::dialog::dialog_utils::uri_resolver::resolve_uri_to_socketaddr(&uri)
            .await
            .ok_or_else(|| SessionError::network_error(&format!("Failed to resolve target address: {}", target_uri)))?;

        tracing::info!("Sending SUBSCRIBE for event '{}' to {} ({})", event_type, target_uri, destination);

        // Send the SUBSCRIBE request via dialog-core
        let response = self.dialog_coordinator.dialog_api()
            .send_non_dialog_request(request, destination, Duration::from_secs(32))
            .await
            .map_err(|e| SessionError::internal(&format!("SUBSCRIBE failed: {}", e)))?;

        let status = response.status_code();
        if status >= 300 {
            return Err(SessionError::SipError(format!(
                "SUBSCRIBE rejected with {} {}",
                status,
                response.reason_phrase()
            )));
        }

        Ok(SubscriptionHandle {
            dialog_id: call_id,
            event_type: event_type.to_string(),
            expires_at: std::time::Instant::now() + Duration::from_secs(u64::from(expires)),
        })
    }
    
    async fn send_raw_request(
        &self,
        request: rvoip_sip_core::Request,
        timeout: Duration,
    ) -> Result<SipResponse> {
        if !self.config.enable_sip_client {
            return Err(SessionError::NotSupported {
                feature: "SIP client operations".to_string(),
                reason: "enable_sip_client must be set to true in configuration".to_string(),
            });
        }
        
        // Extract destination from the request URI
        let request_uri = request.uri().to_string();
        let uri: rvoip_sip_core::Uri = request_uri.parse()
            .map_err(|e| SessionError::invalid_uri(&format!("Invalid request URI: {}", e)))?;
        
        // Resolve URI to socket address
        let destination = rvoip_dialog_core::dialog::dialog_utils::uri_resolver::resolve_uri_to_socketaddr(&uri)
            .await
            .ok_or_else(|| SessionError::network_error(&format!("Failed to resolve request URI: {}", request_uri)))?;
        
        tracing::info!("Sending {} request to {} ({})", request.method(), request_uri, destination);
        
        // Send the request via dialog-core
        let response = self.dialog_coordinator.dialog_api()
            .send_non_dialog_request(request, destination, timeout)
            .await
            .map_err(|e| SessionError::internal(&format!("Request failed: {}", e)))?;
        
        // Convert to SipResponse
        Ok(SipResponse {
            status_code: response.status_code(),
            reason_phrase: response.reason_phrase().to_string(),
            headers: HashMap::new(), // TODO: Extract headers if needed
            body: if response.body().is_empty() {
                None
            } else {
                Some(String::from_utf8_lossy(response.body()).to_string())
            },
        })
    }
} 