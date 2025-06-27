# Media Preferences Integration Guide

This document explains how client-core should integrate with the enhanced session-core API to properly handle media preferences and codec negotiation.

## The Problem

Currently, client-core has extensive media configuration capabilities but cannot effectively use them because:
1. Session-core's `SessionManagerBuilder` didn't accept media preferences
2. Client-core always passes `None` for SDP when accepting incoming calls
3. Session-core uses default codecs when generating SDP answers

## The Solution

We've enhanced the session-core API layer to accept media preferences through the `SessionManagerBuilder`:

```rust
use rvoip_session_core::api::*;

// Create media configuration
let media_config = MediaConfig {
    preferred_codecs: vec!["opus".to_string(), "PCMU".to_string()],
    echo_cancellation: true,
    noise_suppression: true,
    auto_gain_control: true,
    dtmf_enabled: true,
    max_bandwidth_kbps: Some(128),
    preferred_ptime: Some(20),
    custom_sdp_attributes: HashMap::new(),
};

// Configure SessionManager with media preferences
let coordinator = SessionManagerBuilder::new()
    .with_sip_port(5060)
    .with_local_address("sip:alice@example.com")
    .with_media_config(media_config)  // NEW: Pass media preferences
    .with_handler(handler)
    .build()
    .await?;
```

## Integration Steps for client-core

### 1. Update ClientManager Creation

Modify `ClientManager::new()` to pass media preferences to SessionManagerBuilder:

```rust
// In client/manager.rs
impl ClientManager {
    pub async fn new(config: ClientConfig) -> ClientResult<Arc<Self>> {
        // Convert client MediaConfig to session-core MediaConfig
        let session_media_config = rvoip_session_core::api::MediaConfig {
            preferred_codecs: config.media.preferred_codecs.clone(),
            echo_cancellation: config.media.echo_cancellation,
            noise_suppression: config.media.noise_suppression,
            auto_gain_control: config.media.auto_gain_control,
            dtmf_enabled: config.media.dtmf_enabled,
            max_bandwidth_kbps: config.media.max_bandwidth_kbps,
            preferred_ptime: config.media.preferred_ptime,
            custom_sdp_attributes: config.media.custom_sdp_attributes.clone(),
        };
        
        // Create coordinator with media preferences
        let coordinator = SessionManagerBuilder::new()
            .with_sip_port(config.local_sip_addr.port())
            .with_local_address(format!("sip:user@{}", config.local_sip_addr))
            .with_media_ports(config.media.rtp_port_start, config.media.rtp_port_end)
            .with_media_config(session_media_config)  // Pass media config
            .with_handler(handler)
            .build()
            .await?;
            
        // ... rest of initialization
    }
}
```

### 2. Remove Manual SDP Generation

Since session-core now handles SDP generation with preferences, client-core should:

1. **For incoming calls**: Continue passing `None` for SDP in `CallAction::Accept`
2. **For outgoing calls**: Let session-core generate the SDP offer with preferences

### 3. Benefits

With this integration:
- Codec preferences are automatically applied to all SDP generation
- Audio processing settings are consistently configured
- Custom SDP attributes are included in all offers/answers
- No need for client-core to manually generate SDP

## Example Usage

```rust
// Configure client with media preferences
let client = ClientBuilder::new()
    .local_address("127.0.0.1:5060".parse()?)
    .with_media(|m| m
        .codecs(vec!["opus", "G722", "PCMU"])
        .require_srtp(false)
        .echo_cancellation(true)
        .max_bandwidth_kbps(128)
    )
    .build()
    .await?;

// When accepting incoming calls, preferences are automatically used
client.accept_call(&call_id).await?;  // SDP answer includes opus, G722, PCMU

// When making outgoing calls, preferences are automatically used
let call_id = client.make_call("sip:bob@example.com").await?;  // SDP offer includes preferences
```

## Testing

To verify the integration:

1. Make a test call with specific codec preferences
2. Check the generated SDP includes the preferred codecs in order
3. Verify audio processing settings are applied
4. Confirm custom SDP attributes appear in the SDP

## Migration Path

1. Update session-core dependency to include MediaConfig support
2. Modify ClientManager to pass media config to SessionManagerBuilder
3. Test with existing client applications
4. Remove any manual SDP generation code that's no longer needed 