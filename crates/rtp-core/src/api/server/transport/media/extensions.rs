//! Header extensions functionality
//!
//! This module handles RTP header extensions.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

use crate::api::common::error::MediaTransportError;
use crate::api::common::extension::ExtensionFormat;
use crate::api::server::transport::HeaderExtension;

/// Check if header extensions are enabled
pub async fn is_header_extensions_enabled(
    header_extensions_enabled: &Arc<RwLock<bool>>,
) -> Result<bool, MediaTransportError> {
    Ok(*header_extensions_enabled.read().await)
}

/// Enable header extensions with the specified format
pub async fn enable_header_extensions(
    header_extensions_enabled: &Arc<RwLock<bool>>,
    header_extension_format: &Arc<RwLock<ExtensionFormat>>,
    format: ExtensionFormat,
) -> Result<bool, MediaTransportError> {
    // Check if already enabled
    if *header_extensions_enabled.read().await {
        return Ok(true);
    }
    
    // Set format and enabled flag
    *header_extension_format.write().await = format;
    *header_extensions_enabled.write().await = true;
    
    debug!("Enabled header extensions with format: {:?}", format);
    Ok(true)
}

/// Configure a header extension mapping
pub async fn configure_header_extension(
    header_extensions_enabled: &Arc<RwLock<bool>>,
    header_extension_format: &Arc<RwLock<ExtensionFormat>>,
    header_extension_mappings: &Arc<RwLock<HashMap<u8, String>>>,
    id: u8,
    uri: String,
) -> Result<(), MediaTransportError> {
    // Check if header extensions are enabled
    if !*header_extensions_enabled.read().await {
        return Err(MediaTransportError::ConfigError("Header extensions are not enabled".to_string()));
    }
    
    // Check if ID is valid for the current format
    let format = *header_extension_format.read().await;
    match format {
        ExtensionFormat::OneByte => {
            if id < 1 || id > 14 {
                return Err(MediaTransportError::ConfigError(
                    format!("Invalid extension ID for one-byte format: {} (must be 1-14)", id)
                ));
            }
        },
        ExtensionFormat::TwoByte => {
            if id < 1 || id > 255 {
                return Err(MediaTransportError::ConfigError(
                    format!("Invalid extension ID for two-byte format: {} (must be 1-255)", id)
                ));
            }
        },
        ExtensionFormat::Legacy => {
            // No specific validation for legacy format
        },
    }
    
    // Add mapping
    let mut mappings = header_extension_mappings.write().await;
    mappings.insert(id, uri.clone());
    
    debug!("Configured header extension ID {} -> URI: {}", id, uri);
    Ok(())
}

/// Configure multiple header extension mappings
pub async fn configure_header_extensions(
    header_extensions_enabled: &Arc<RwLock<bool>>,
    header_extension_format: &Arc<RwLock<ExtensionFormat>>,
    header_extension_mappings: &Arc<RwLock<HashMap<u8, String>>>,
    mappings: HashMap<u8, String>,
) -> Result<(), MediaTransportError> {
    // Configure each mapping
    for (id, uri) in mappings {
        configure_header_extension(
            header_extensions_enabled,
            header_extension_format,
            header_extension_mappings,
            id, 
            uri
        ).await?;
    }
    
    Ok(())
}

/// Add header extension for a client
pub async fn add_header_extension_for_client(
    header_extensions_enabled: &Arc<RwLock<bool>>,
    clients: &Arc<RwLock<HashMap<String, crate::api::server::transport::core::connection::ClientConnection>>>,
    pending_extensions: &Arc<RwLock<HashMap<String, Vec<HeaderExtension>>>>,
    client_id: &str,
    extension: HeaderExtension,
) -> Result<(), MediaTransportError> {
    // Check if header extensions are enabled
    if !*header_extensions_enabled.read().await {
        return Err(MediaTransportError::ConfigError("Header extensions are not enabled".to_string()));
    }
    
    // Make sure the client exists
    let clients_guard = clients.read().await;
    if !clients_guard.contains_key(client_id) {
        return Err(MediaTransportError::ClientNotFound(client_id.to_string()));
    }
    drop(clients_guard);
    
    // Add extension to pending list for this client
    let mut pending = pending_extensions.write().await;
    
    // Get or create the pending list for this client
    let client_pending = pending.entry(client_id.to_string()).or_insert_with(Vec::new);
    
    // Add the extension
    client_pending.push(extension);
    
    Ok(())
}

/// Add header extension for all clients
pub async fn add_header_extension_for_all_clients(
    header_extensions_enabled: &Arc<RwLock<bool>>,
    clients: &Arc<RwLock<HashMap<String, crate::api::server::transport::core::connection::ClientConnection>>>,
    pending_extensions: &Arc<RwLock<HashMap<String, Vec<HeaderExtension>>>>,
    extension: HeaderExtension,
) -> Result<(), MediaTransportError> {
    // Check if header extensions are enabled
    if !*header_extensions_enabled.read().await {
        return Err(MediaTransportError::ConfigError("Header extensions are not enabled".to_string()));
    }
    
    // Get all client IDs
    let clients_guard = clients.read().await;
    let client_ids: Vec<String> = clients_guard.keys().cloned().collect();
    drop(clients_guard);
    
    // Add extension for each client
    for client_id in client_ids {
        add_header_extension_for_client(
            header_extensions_enabled,
            clients,
            pending_extensions,
            &client_id,
            extension.clone()
        ).await?;
    }
    
    Ok(())
}

/// Add audio level extension for a client
pub async fn add_audio_level_extension_for_client(
    header_extensions_enabled: &Arc<RwLock<bool>>,
    header_extension_mappings: &Arc<RwLock<HashMap<u8, String>>>,
    clients: &Arc<RwLock<HashMap<String, crate::api::server::transport::core::connection::ClientConnection>>>,
    pending_extensions: &Arc<RwLock<HashMap<String, Vec<HeaderExtension>>>>,
    client_id: &str,
    voice_activity: bool,
    level: u8,
) -> Result<(), MediaTransportError> {
    // Check if header extensions are enabled
    if !*header_extensions_enabled.read().await {
        return Err(MediaTransportError::ConfigError("Header extensions are not enabled".to_string()));
    }
    
    // Clamp level to 0-127
    let level = level.min(127);
    
    // For audio level, the format is:
    // - Bit 7 (MSB): Voice activity flag (0 = active, 1 = inactive)
    // - Bits 0-6: Level (0-127 dB)
    let data = if voice_activity {
        // For active voice, the MSB is 0
        vec![level]
    } else {
        // For inactive voice, the MSB is 1
        vec![level | 0x80]
    };
    
    // Find the audio level extension ID
    let audio_level_id = {
        let mappings = header_extension_mappings.read().await;
        
        // Look for urn:ietf:params:rtp-hdrext:ssrc-audio-level in mappings
        let mut id = None;
        for (mapping_id, uri) in mappings.iter() {
            if uri == "urn:ietf:params:rtp-hdrext:ssrc-audio-level" {
                id = Some(*mapping_id);
                break;
            }
        }
        
        // Default to ID 1 if not found
        id.unwrap_or(1)
    };
    
    // Create extension
    let extension = HeaderExtension {
        id: audio_level_id,
        uri: "urn:ietf:params:rtp-hdrext:ssrc-audio-level".to_string(),
        data,
    };
    
    // Add extension for client
    add_header_extension_for_client(
        header_extensions_enabled,
        clients,
        pending_extensions,
        client_id,
        extension
    ).await
}

/// Add audio level extension for all clients
pub async fn add_audio_level_extension_for_all_clients(
    header_extensions_enabled: &Arc<RwLock<bool>>,
    header_extension_mappings: &Arc<RwLock<HashMap<u8, String>>>,
    clients: &Arc<RwLock<HashMap<String, crate::api::server::transport::core::connection::ClientConnection>>>,
    pending_extensions: &Arc<RwLock<HashMap<String, Vec<HeaderExtension>>>>,
    voice_activity: bool,
    level: u8,
) -> Result<(), MediaTransportError> {
    // Check if header extensions are enabled
    if !*header_extensions_enabled.read().await {
        return Err(MediaTransportError::ConfigError("Header extensions are not enabled".to_string()));
    }
    
    // Get all client IDs
    let clients_guard = clients.read().await;
    let client_ids: Vec<String> = clients_guard.keys().cloned().collect();
    drop(clients_guard);
    
    // Add audio level extension for each client
    for client_id in client_ids {
        add_audio_level_extension_for_client(
            header_extensions_enabled,
            header_extension_mappings,
            clients,
            pending_extensions,
            &client_id,
            voice_activity,
            level
        ).await?;
    }
    
    Ok(())
}

/// Get received header extensions for a client
pub async fn get_received_header_extensions(
    header_extensions_enabled: &Arc<RwLock<bool>>,
    received_extensions: &Arc<RwLock<HashMap<String, Vec<HeaderExtension>>>>,
    client_id: &str,
) -> Result<Vec<HeaderExtension>, MediaTransportError> {
    // Check if header extensions are enabled
    if !*header_extensions_enabled.read().await {
        return Err(MediaTransportError::ConfigError("Header extensions are not enabled".to_string()));
    }
    
    // Get extensions for this client
    let extensions = received_extensions.read().await;
    let client_extensions = extensions.get(client_id).cloned().unwrap_or_default();
    
    Ok(client_extensions)
}

/// Get audio level header extension for a client
pub async fn get_received_audio_level(
    header_extensions_enabled: &Arc<RwLock<bool>>,
    header_extension_mappings: &Arc<RwLock<HashMap<u8, String>>>,
    received_extensions: &Arc<RwLock<HashMap<String, Vec<HeaderExtension>>>>,
    client_id: &str,
) -> Result<Option<(bool, u8)>, MediaTransportError> {
    // Check if header extensions are enabled
    if !*header_extensions_enabled.read().await {
        return Err(MediaTransportError::ConfigError("Header extensions are not enabled".to_string()));
    }
    
    // Get all extensions for this client
    let client_extensions = get_received_header_extensions(
        header_extensions_enabled,
        received_extensions,
        client_id
    ).await?;
    
    // Look for audio level extension (typically ID 1)
    for ext in client_extensions {
        // Find by URI if mapped, or by common ID 1
        let is_audio_level = {
            let mappings = header_extension_mappings.read().await;
            mappings.get(&ext.id).map(|uri| uri == "urn:ietf:params:rtp-hdrext:ssrc-audio-level").unwrap_or(ext.id == 1)
        };
        
        if is_audio_level && !ext.data.is_empty() {
            // Parse audio level data
            let byte = ext.data[0];
            
            // Extract voice activity flag (0 = active, 1 = inactive)
            let voice_activity = (byte & 0x80) == 0;
            
            // Extract level (0-127 dB)
            let level = byte & 0x7F;
            
            return Ok(Some((voice_activity, level)));
        }
    }
    
    Ok(None)
}

/// Clear all pending header extensions
pub async fn clear_pending_header_extensions(
    pending_extensions: &Arc<RwLock<HashMap<String, Vec<HeaderExtension>>>>,
) -> Result<(), MediaTransportError> {
    let mut pending = pending_extensions.write().await;
    pending.clear();
    
    debug!("Cleared all pending header extensions");
    Ok(())
} 