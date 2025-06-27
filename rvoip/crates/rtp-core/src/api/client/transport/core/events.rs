//! Event handling for client transport
//!
//! This module handles event subscription and notification for the media transport client,
//! including connection, disconnection, and media events.

use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::api::common::events::{MediaTransportEvent, MediaEventCallback};
use crate::api::common::error::MediaTransportError;

/// Register a callback for media transport events
///
/// This function registers a callback function that will be invoked when
/// media transport events occur, such as quality changes or stream status updates.
pub async fn register_event_callback(
    callbacks: &Arc<Mutex<Vec<MediaEventCallback>>>,
    callback: MediaEventCallback,
) -> Result<(), MediaTransportError> {
    // Placeholder for the extracted event callback registration
    let mut callbacks_guard = callbacks.lock().await;
    callbacks_guard.push(callback);
    debug!("Registered new event callback, total callbacks: {}", callbacks_guard.len());
    Ok(())
}

/// Register a callback for connection events
///
/// This function registers a callback function that will be invoked when
/// the client successfully connects to the remote peer.
pub async fn register_connect_callback(
    callbacks: &Arc<Mutex<Vec<Box<dyn Fn() + Send + Sync>>>>,
    callback: Box<dyn Fn() + Send + Sync>,
) -> Result<(), MediaTransportError> {
    // Placeholder for the extracted connect callback registration
    let mut callbacks_guard = callbacks.lock().await;
    callbacks_guard.push(callback);
    debug!("Registered new connect callback, total callbacks: {}", callbacks_guard.len());
    Ok(())
}

/// Register a callback for disconnection events
///
/// This function registers a callback function that will be invoked when
/// the client disconnects from the remote peer.
pub async fn register_disconnect_callback(
    callbacks: &Arc<Mutex<Vec<Box<dyn Fn() + Send + Sync>>>>,
    callback: Box<dyn Fn() + Send + Sync>,
) -> Result<(), MediaTransportError> {
    // Placeholder for the extracted disconnect callback registration
    let mut callbacks_guard = callbacks.lock().await;
    callbacks_guard.push(callback);
    debug!("Registered new disconnect callback, total callbacks: {}", callbacks_guard.len());
    Ok(())
}

/// Notify event subscribers of a media transport event
///
/// This function notifies all registered event callbacks about a media transport event.
pub async fn notify_event(
    callbacks: &Arc<Mutex<Vec<MediaEventCallback>>>,
    event: MediaTransportEvent,
) -> Result<(), MediaTransportError> {
    // Placeholder for event notification functionality
    let callbacks_guard = callbacks.lock().await;
    for callback in &*callbacks_guard {
        // MediaEventCallback is a non-async Fn type, so just call it directly
        let event_clone = event.clone();
        // Call the callback synchronously - it's just a regular function
        callback(event_clone);
    }
    Ok(())
} 