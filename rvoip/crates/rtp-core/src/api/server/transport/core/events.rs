//! Event handling
//!
//! This module handles event subscription and callback management.

use std::sync::Arc;
use tokio::sync::RwLock;
use crate::api::common::events::{MediaEventCallback, MediaTransportEvent};
use crate::api::common::error::MediaTransportError;
use crate::api::server::transport::ClientInfo;

/// Register a callback for media transport events
pub async fn on_event(
    callback: MediaEventCallback,
    event_callbacks: &Arc<RwLock<Vec<MediaEventCallback>>>,
) -> Result<(), MediaTransportError> {
    let mut callbacks = event_callbacks.write().await;
    callbacks.push(callback);
    Ok(())
}

/// Register a callback for client connected events
pub async fn on_client_connected(
    callback: Box<dyn Fn(ClientInfo) + Send + Sync>,
    client_connected_callbacks: &Arc<RwLock<Vec<Box<dyn Fn(ClientInfo) + Send + Sync>>>>,
) -> Result<(), MediaTransportError> {
    let mut callbacks = client_connected_callbacks.write().await;
    callbacks.push(callback);
    Ok(())
}

/// Register a callback for client disconnected events
pub async fn on_client_disconnected(
    callback: Box<dyn Fn(ClientInfo) + Send + Sync>,
    client_disconnected_callbacks: &Arc<RwLock<Vec<Box<dyn Fn(ClientInfo) + Send + Sync>>>>,
) -> Result<(), MediaTransportError> {
    let mut callbacks = client_disconnected_callbacks.write().await;
    callbacks.push(callback);
    Ok(())
} 