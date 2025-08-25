//! Subscription manager for handling SIP event subscriptions
//!
//! This module provides the SubscriptionManager that handles subscription
//! lifecycle, refresh timers, and NOTIFY processing according to RFC 6665.

use std::sync::Arc;
use std::time::Duration;
use std::collections::HashMap;
use std::net::SocketAddr;
use tokio::sync::{RwLock, mpsc};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn, error};
use dashmap::DashMap;

use rvoip_sip_core::{
    Request, Response, Method, StatusCode,
    TypedHeader, HeaderName,
    types::{
        event::Event,
        expires::Expires,
        subscription_state::SubscriptionState as SubscriptionStateHeader,
    },
    builder::{SimpleRequestBuilder, SimpleResponseBuilder},
};

use crate::dialog::{DialogId, Dialog, DialogState, SubscriptionState, SubscriptionTerminationReason};
use crate::errors::{DialogError, DialogResult};
use crate::events::DialogEvent;
use super::event_package::EventPackage;

/// Manages SIP event subscriptions
pub struct SubscriptionManager {
    /// Shared dialog store from DialogManager
    dialogs: Arc<DashMap<DialogId, Dialog>>,
    
    /// Shared dialog lookup from DialogManager
    dialog_lookup: Arc<DashMap<String, DialogId>>,
    
    /// Refresh timer handles by dialog ID
    refresh_timers: Arc<DashMap<DialogId, JoinHandle<()>>>,
    
    /// Registered event packages
    #[allow(dead_code)]
    event_packages: Arc<DashMap<String, Box<dyn EventPackage>>>,
    
    /// Channel for sending dialog events
    event_tx: mpsc::Sender<DialogEvent>,
    
    /// Channel for receiving internal commands
    command_rx: Arc<RwLock<Option<mpsc::Receiver<SubscriptionCommand>>>>,
    
    /// Channel for sending internal commands
    command_tx: mpsc::Sender<SubscriptionCommand>,
}

/// Internal commands for subscription management
#[derive(Debug)]
enum SubscriptionCommand {
    RefreshSubscription(DialogId),
    TerminateSubscription(DialogId, Option<SubscriptionTerminationReason>),
}

impl SubscriptionManager {
    /// Create a new subscription manager with shared dialog stores
    pub fn new(
        dialogs: Arc<DashMap<DialogId, Dialog>>,
        dialog_lookup: Arc<DashMap<String, DialogId>>,
        event_tx: mpsc::Sender<DialogEvent>,
    ) -> Self {
        let (command_tx, command_rx) = mpsc::channel(100);
        
        let mut manager = Self {
            dialogs,
            dialog_lookup,
            refresh_timers: Arc::new(DashMap::new()),
            event_packages: Arc::new(DashMap::new()),
            event_tx,
            command_rx: Arc::new(RwLock::new(Some(command_rx))),
            command_tx,
        };
        
        // Register default event packages
        manager.register_default_packages();
        
        manager
    }
    
    /// Register default event packages
    fn register_default_packages(&mut self) {
        use super::event_package::{PresencePackage, DialogPackage, MessageSummaryPackage, ReferPackage};
        
        self.register_event_package(Box::new(PresencePackage));
        self.register_event_package(Box::new(DialogPackage));
        self.register_event_package(Box::new(MessageSummaryPackage));
        self.register_event_package(Box::new(ReferPackage));
    }
    
    /// Register an event package
    pub fn register_event_package(&mut self, package: Box<dyn EventPackage>) {
        let name = package.name().to_string();
        self.event_packages.insert(name, package);
    }
    
    /// Handle incoming SUBSCRIBE request
    pub async fn handle_subscribe(
        &self,
        request: Request,
        source: SocketAddr,
        local_addr: SocketAddr,
    ) -> DialogResult<(Response, Option<DialogId>)> {
        // Extract Event header
        let event = request.header(&HeaderName::Event)
            .and_then(|h| match h {
                TypedHeader::Event(e) => Some(e),
                _ => None,
            })
            .ok_or_else(|| DialogError::protocol_error("SUBSCRIBE requires Event header"))?;
        
        let event_package = event.event_type.to_string();
        
        // Check if event package is supported
        if !self.event_packages.contains_key(&event_package) {
            // Return 489 Bad Event
            // Build 489 Bad Event response
            let supported_events = self.get_supported_events();
            let event_refs: Vec<&str> = supported_events.iter().map(|s| s.as_str()).collect();
            let response = self.build_response_from_request(
                StatusCode::BadEvent,
                None,
                &request,
            ).allow_events(&event_refs).build();
            return Ok((response, None));
        }
        
        // Get the event package handler
        let package = self.event_packages.get(&event_package)
            .ok_or_else(|| DialogError::protocol_error("Event package not found"))?;
        
        // Extract Expires header
        let expires = request.header(&HeaderName::Expires)
            .and_then(|h| match h {
                TypedHeader::Expires(e) => Some(e.0),
                _ => None,
            })
            .unwrap_or(package.default_expires().as_secs() as u32);
        
        // Validate expires against package limits
        let min_expires = package.min_expires().as_secs() as u32;
        let max_expires = package.max_expires().as_secs() as u32;
        
        if expires < min_expires && expires != 0 {
            // Return 423 Interval Too Brief
            // Return 423 Interval Too Brief
            let response = self.build_response_from_request(
                StatusCode::IntervalTooBrief,
                None,
                &request,
            ).min_expires(min_expires).build();
            return Ok((response, None));
        }
        
        let adjusted_expires = if expires > max_expires {
            max_expires
        } else {
            expires
        };
        
        // Create subscription dialog
        let dialog_id = DialogId::new();
        let local_tag = format!("{:08x}", rand::random::<u32>());
        
        // Extract dialog information from request
        let call_id = request.call_id()
            .map(|c| c.value().to_string())
            .unwrap_or_else(|| format!("sub-{}", uuid::Uuid::new_v4()));
        
        let local_uri = request.to()
            .map(|t| t.uri.clone())
            .ok_or_else(|| DialogError::protocol_error("Missing To header"))?;
        
        let remote_uri = request.from()
            .map(|f| f.uri.clone())
            .ok_or_else(|| DialogError::protocol_error("Missing From header"))?;
        
        let remote_tag = request.from()
            .and_then(|f| f.tag().map(|t| t.to_string()));
        
        // Create the dialog
        let mut dialog = Dialog::new(
            call_id.clone(),
            local_uri,
            remote_uri,
            Some(local_tag.clone()),
            remote_tag,
            false, // Not initiator for incoming SUBSCRIBE
        );
        dialog.id = dialog_id.clone();
        dialog.state = DialogState::Early; // Early until we send 200 OK
        dialog.remote_cseq = request.cseq()
            .map(|c| c.seq)
            .unwrap_or(1);
        
        // Set subscription-specific fields
        dialog.subscription_state = Some(if adjusted_expires > 0 {
            SubscriptionState::Pending
        } else {
            SubscriptionState::Terminated {
                reason: Some(SubscriptionTerminationReason::ClientRequested),
            }
        });
        dialog.event_package = Some(event_package.clone());
        dialog.event_id = event.id.clone();
        
        // Store the dialog
        self.dialogs.insert(dialog_id.clone(), dialog.clone());
        
        // Add to lookup table
        let lookup_key = format!("{}:{}:{}", 
            dialog.call_id,
            dialog.local_tag.as_deref().unwrap_or(""),
            dialog.remote_tag.as_deref().unwrap_or("")
        );
        self.dialog_lookup.insert(lookup_key, dialog_id.clone());
        
        // Start refresh timer if subscription is active
        if adjusted_expires > 0 {
            self.start_refresh_timer(dialog_id.clone(), Duration::from_secs(adjusted_expires as u64)).await;
        }
        
        // Build 200 OK response
        // Build 200 OK response
        let mut response = self.build_response_from_request(
            StatusCode::Ok,
            None,
            &request,
        ).expires(adjusted_expires);
        
        // Add To tag for dialog creation
        if let Some(to) = request.to() {
            response = response.to(
                to.display_name.as_deref().unwrap_or(""),
                &to.uri.to_string(),
                Some(&local_tag),
            );
        }
        
        let response = response.build();
        
        // Emit subscription created event for session-core to handle
        // Session-core is responsible for generating and sending initial NOTIFY
        let _ = self.event_tx.send(DialogEvent::SubscriptionCreated {
            dialog_id: dialog_id.clone(),
            event_package,
            expires: Duration::from_secs(adjusted_expires as u64),
        }).await;
        
        Ok((response, Some(dialog_id)))
    }
    
    /// Handle incoming NOTIFY request
    pub async fn handle_notify(
        &self,
        request: Request,
        _source: SocketAddr,
    ) -> DialogResult<Response> {
        // Extract Subscription-State header
        let subscription_state = request.header(&HeaderName::SubscriptionState)
            .and_then(|h| match h {
                TypedHeader::SubscriptionState(s) => Some(s),
                _ => None,
            })
            .ok_or_else(|| DialogError::protocol_error("NOTIFY requires Subscription-State header"))?;
        
        // Find subscription by Call-ID and tags
        let call_id = request.call_id()
            .map(|c| c.value().to_string())
            .ok_or_else(|| DialogError::protocol_error("Missing Call-ID"))?;
        
        let to_tag = request.to()
            .and_then(|t| t.tag().map(|s| s.to_string()));
        
        let from_tag = request.from()
            .and_then(|f| f.tag().map(|s| s.to_string()));
        
        // Find matching dialog using lookup
        let lookup_key = format!("{}:{}:{}", 
            call_id,
            to_tag.as_deref().unwrap_or(""),
            from_tag.as_deref().unwrap_or("")
        );
        
        if let Some(dialog_id_entry) = self.dialog_lookup.get(&lookup_key) {
            let dialog_id = dialog_id_entry.value().clone();
            
            if let Some(mut dialog) = self.dialogs.get_mut(&dialog_id) {
                // Update subscription state
                let new_state = SubscriptionState::from_header_value(&subscription_state.to_string());
                dialog.subscription_state = Some(new_state.clone());
                
                // Emit NOTIFY received event
                let _ = self.event_tx.send(DialogEvent::NotifyReceived {
                    dialog_id: dialog_id.clone(),
                    state: new_state.clone(),
                    body: if !request.body().is_empty() {
                        Some(request.body().to_vec())
                    } else {
                        None
                    },
                }).await;
                
                // If subscription is terminated, clean up
                if new_state.is_terminated() {
                    drop(dialog); // Release lock before cleanup
                    self.cleanup_subscription(&dialog_id).await;
                }
            }
        }
        
        // Always respond 200 OK to NOTIFY (RFC 6665)
        // Always respond 200 OK to NOTIFY (RFC 6665)
        let response = self.build_response_from_request(
            StatusCode::Ok,
            None,
            &request,
        ).build();
        
        Ok(response)
    }
    
    /// Mark subscription as active after initial NOTIFY is sent
    pub async fn activate_subscription(&self, dialog_id: &DialogId) -> DialogResult<()> {
        if let Some(mut dialog) = self.dialogs.get_mut(dialog_id) {
            if let Some(SubscriptionState::Pending) = &dialog.subscription_state {
                dialog.subscription_state = Some(SubscriptionState::Active {
                    remaining_duration: Duration::from_secs(3600),
                    original_duration: Duration::from_secs(3600),
                });
                dialog.state = DialogState::Confirmed;
                debug!("Subscription {} activated", dialog_id);
            }
        }
        Ok(())
    }
    
    /// Start refresh timer for a subscription
    async fn start_refresh_timer(&self, dialog_id: DialogId, duration: Duration) {
        // Cancel existing timer if any
        if let Some((_key, handle)) = self.refresh_timers.remove(&dialog_id) {
            handle.abort();
        }
        
        // Calculate refresh time (30 seconds before expiry)
        let refresh_time = if duration > Duration::from_secs(30) {
            duration - Duration::from_secs(30)
        } else {
            duration / 2
        };
        
        let command_tx = self.command_tx.clone();
        let dialog_id_clone = dialog_id.clone();
        
        let handle = tokio::spawn(async move {
            tokio::time::sleep(refresh_time).await;
            let _ = command_tx.send(SubscriptionCommand::RefreshSubscription(dialog_id_clone)).await;
        });
        
        self.refresh_timers.insert(dialog_id, handle);
    }
    
    /// Clean up a terminated subscription
    async fn cleanup_subscription(&self, dialog_id: &DialogId) {
        // Mark dialog as terminated
        if let Some(mut dialog) = self.dialogs.get_mut(dialog_id) {
            dialog.state = DialogState::Terminated;
        }
        
        // Cancel refresh timer
        if let Some((_key, handle)) = self.refresh_timers.remove(dialog_id) {
            handle.abort();
        }
        
        debug!("Cleaned up subscription {}", dialog_id);
    }
    
    /// Build a response from a request, copying necessary headers
    fn build_response_from_request(
        &self,
        status: StatusCode,
        reason: Option<&str>,
        request: &Request,
    ) -> SimpleResponseBuilder {
        let mut builder = SimpleResponseBuilder::new(status, reason);
        
        // Copy required headers
        if let Some(from) = request.from() {
            builder = builder.from(
                from.display_name.as_deref().unwrap_or(""),
                &from.uri.to_string(),
                from.tag().as_deref(),
            );
        }
        
        if let Some(to) = request.to() {
            builder = builder.to(
                to.display_name.as_deref().unwrap_or(""),
                &to.uri.to_string(),
                to.tag().as_deref(),
            );
        }
        
        if let Some(call_id) = request.call_id() {
            builder = builder.call_id(&call_id.value());
        }
        
        if let Some(cseq) = request.cseq() {
            builder = builder.cseq(cseq.seq, cseq.method.clone());
        }
        
        if let Some(via) = request.first_via() {
            if let Some(first_via_header) = via.0.first() {
                let host = format!("{}:{}", first_via_header.sent_by_host, first_via_header.sent_by_port.unwrap_or(5060));
                let transport = &first_via_header.sent_protocol.transport;
                builder = builder.via(
                    &host,
                    transport,
                    first_via_header.branch().as_deref(),
                );
            }
        }
        
        builder
    }
    
    /// Get list of supported event packages
    fn get_supported_events(&self) -> Vec<String> {
        self.event_packages.iter()
            .map(|entry| entry.key().clone())
            .collect()
    }
    
    /// Terminate a subscription
    pub async fn terminate_subscription(
        &self,
        dialog_id: &DialogId,
        reason: Option<SubscriptionTerminationReason>,
    ) -> DialogResult<()> {
        if let Some(mut dialog) = self.dialogs.get_mut(dialog_id) {
            dialog.subscription_state = Some(SubscriptionState::Terminated { reason: reason.clone() });
            
            // Emit event for session-core to send termination NOTIFY
            let _ = self.event_tx.send(DialogEvent::SubscriptionTerminated {
                dialog_id: dialog_id.clone(),
                reason: reason.map(|r| r.to_string()),
            }).await;
            
            drop(dialog); // Release lock before cleanup
            
            // Clean up
            self.cleanup_subscription(dialog_id).await;
        }
        
        Ok(())
    }
}

// Debug implementation for SubscriptionManager
impl std::fmt::Debug for SubscriptionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubscriptionManager")
            .field("dialogs", &self.dialogs.len())
            .field("refresh_timers", &self.refresh_timers.len())
            .field("event_packages", &self.event_packages.len())
            .finish()
    }
}

// Clone implementation for SubscriptionManager
impl Clone for SubscriptionManager {
    fn clone(&self) -> Self {
        Self {
            dialogs: self.dialogs.clone(),
            dialog_lookup: self.dialog_lookup.clone(),
            refresh_timers: self.refresh_timers.clone(),
            event_packages: self.event_packages.clone(),
            event_tx: self.event_tx.clone(),
            command_rx: self.command_rx.clone(),
            command_tx: self.command_tx.clone(),
        }
    }
}