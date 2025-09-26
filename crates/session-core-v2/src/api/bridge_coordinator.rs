//! Bridge Coordinator - Manages paired B2BUA sessions
//!
//! Implements the "Linked Sessions" approach for coordinating
//! state between two independent call legs in a B2BUA bridge.

use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;
use dashmap::DashMap;
use uuid::Uuid;
use tracing::{debug, info, warn, error};

use crate::state_table::types::SessionId;
use crate::types::CallState;
use crate::errors::{Result, SessionError};
use crate::adapters::MediaMode;
use crate::api::UnifiedCoordinator;

/// Bridge event for notification
#[derive(Debug, Clone)]
pub enum BridgeEvent {
    /// Bridge created
    BridgeCreated {
        inbound_id: SessionId,
        outbound_id: SessionId,
        media_mode: MediaMode,
    },
    /// Bridge destroyed
    BridgeDestroyed {
        session_id: SessionId,
    },
    /// Leg event occurred
    LegEvent(LegEvent),
}

/// Event from one leg that may affect the other
#[derive(Debug, Clone)]
pub enum LegEvent {
    /// Call state changed
    StateChanged {
        session_id: SessionId,
        new_state: CallState,
    },

    /// Media event
    MediaEvent {
        session_id: SessionId,
        event: MediaEventType,
    },

    /// DTMF received
    DtmfReceived {
        session_id: SessionId,
        digit: char,
    },

    /// Call terminated
    Terminated {
        session_id: SessionId,
        reason: String,
    },

    /// Hold/Resume
    OnHold {
        session_id: SessionId,
    },
    Resumed {
        session_id: SessionId,
    },

    /// Re-INVITE for renegotiation
    ReinviteReceived {
        session_id: SessionId,
        sdp: String,
    },

    /// Transfer request
    TransferRequested {
        session_id: SessionId,
        target: String,
    },
}

/// Media event types
#[derive(Debug, Clone)]
pub enum MediaEventType {
    /// Media started flowing
    MediaFlowing,

    /// Media stopped
    MediaStopped,

    /// RTP timeout detected
    RtpTimeout,

    /// Codec changed
    CodecChanged { codec: String },

    /// Packet loss detected
    PacketLoss { percentage: f32 },
}

/// State of a B2BUA bridge
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeState {
    /// Setting up the bridge
    Initializing,

    /// Outbound leg is ringing
    Ringing,

    /// Both legs connected and active
    Active,

    /// Bridge is on hold
    OnHold,

    /// Bridge is being torn down
    Terminating,

    /// Bridge has been terminated
    Terminated,
}

/// Metadata about a bridge
#[derive(Debug, Clone)]
pub struct BridgeMetadata {
    /// Unique bridge ID
    pub bridge_id: String,

    /// When the bridge was created
    pub created_at: Instant,

    /// Current bridge state
    pub state: BridgeState,

    /// Media handling mode
    pub media_mode: MediaMode,

    /// Original call ID
    pub call_id: String,

    /// From URI of original caller
    pub from_uri: String,

    /// To URI (original destination)
    pub to_uri: String,

    /// Routed target URI
    pub routed_to: String,

    /// Statistics
    pub stats: BridgeStats,
}

/// Statistics for a bridge
#[derive(Debug, Clone, Default)]
pub struct BridgeStats {
    /// Number of re-INVITEs processed
    pub reinvites: u32,

    /// Number of hold/resume cycles
    pub hold_resume_cycles: u32,

    /// Total DTMF digits forwarded
    pub dtmf_forwarded: u32,

    /// Number of media interruptions
    pub media_interruptions: u32,

    /// Duration in seconds (updated periodically)
    pub duration_secs: u64,
}

/// Coordinates two legs of a B2BUA bridge
pub struct BridgeCoordinator {
    /// Active bridges mapping inbound -> outbound
    bridges: Arc<DashMap<SessionId, SessionId>>,

    /// Reverse mapping outbound -> inbound
    reverse_bridges: Arc<DashMap<SessionId, SessionId>>,

    /// Event channels for each bridge
    event_channels: Arc<DashMap<String, mpsc::Sender<LegEvent>>>,

    /// Bridge metadata indexed by bridge ID
    bridge_metadata: Arc<DashMap<String, BridgeMetadata>>,

    /// Session to bridge ID mapping
    session_to_bridge: Arc<DashMap<SessionId, String>>,

    /// Reference to unified coordinator for making API calls
    coordinator: Option<Arc<UnifiedCoordinator>>,
}

impl BridgeCoordinator {
    /// Create a new bridge coordinator
    pub fn new() -> Self {
        Self {
            bridges: Arc::new(DashMap::new()),
            reverse_bridges: Arc::new(DashMap::new()),
            event_channels: Arc::new(DashMap::new()),
            bridge_metadata: Arc::new(DashMap::new()),
            session_to_bridge: Arc::new(DashMap::new()),
            coordinator: None,
        }
    }

    /// Create with a reference to the unified coordinator
    pub fn with_coordinator(coordinator: Arc<UnifiedCoordinator>) -> Self {
        let mut bc = Self::new();
        bc.coordinator = Some(coordinator);
        bc
    }

    /// Register a new bridge between two sessions
    pub async fn register_bridge(
        &self,
        inbound: SessionId,
        outbound: SessionId,
        media_mode: MediaMode,
        call_id: String,
        from_uri: String,
        to_uri: String,
        routed_to: String,
    ) -> Result<String> {
        // Check if sessions are already bridged
        if self.bridges.contains_key(&inbound) {
            return Err(SessionError::Other(
                format!("Inbound session {} is already bridged", inbound)
            ));
        }
        if self.reverse_bridges.contains_key(&outbound) {
            return Err(SessionError::Other(
                format!("Outbound session {} is already bridged", outbound)
            ));
        }

        // Generate bridge ID
        let bridge_id = format!("bridge-{}", Uuid::new_v4());

        // Store mappings
        self.bridges.insert(inbound.clone(), outbound.clone());
        self.reverse_bridges.insert(outbound.clone(), inbound.clone());
        self.session_to_bridge.insert(inbound.clone(), bridge_id.clone());
        self.session_to_bridge.insert(outbound.clone(), bridge_id.clone());

        // Create event channel for coordination
        let (tx, rx) = mpsc::channel(100);
        self.event_channels.insert(bridge_id.clone(), tx);

        // Store metadata
        let metadata = BridgeMetadata {
            bridge_id: bridge_id.clone(),
            created_at: Instant::now(),
            state: BridgeState::Initializing,
            media_mode,
            call_id,
            from_uri,
            to_uri,
            routed_to,
            stats: BridgeStats::default(),
        };
        self.bridge_metadata.insert(bridge_id.clone(), metadata);

        // Spawn coordination task
        let coordinator = self.clone();
        let bridge_id_c = bridge_id.clone();
        let inbound_c = inbound.clone();
        let outbound_c = outbound.clone();

        tokio::spawn(async move {
            coordinator.coordinate_legs(
                bridge_id_c,
                inbound_c,
                outbound_c,
                rx
            ).await;
        });

        info!("Registered B2BUA bridge {} between {} <-> {}",
              bridge_id, inbound, outbound);

        Ok(bridge_id)
    }

    /// Main coordination loop for a bridge
    async fn coordinate_legs(
        &self,
        bridge_id: String,
        inbound: SessionId,
        outbound: SessionId,
        mut rx: mpsc::Receiver<LegEvent>,
    ) {
        info!("Starting coordination for bridge {}", bridge_id);

        // Update state to Active once both legs are ready
        if let Some(mut metadata) = self.bridge_metadata.get_mut(&bridge_id) {
            metadata.state = BridgeState::Active;
        }

        while let Some(event) = rx.recv().await {
            debug!("Bridge {} received event: {:?}", bridge_id, event);

            match event {
                LegEvent::Terminated { session_id, reason } => {
                    // One leg terminated - terminate the other
                    let other_leg = if session_id == inbound {
                        &outbound
                    } else {
                        &inbound
                    };

                    info!("Bridge {}: Leg {} terminated ({}), terminating other leg {}",
                          bridge_id, session_id, reason, other_leg);

                    // Update state
                    if let Some(mut metadata) = self.bridge_metadata.get_mut(&bridge_id) {
                        metadata.state = BridgeState::Terminating;
                    }

                    // Terminate other leg through coordinator
                    if let Some(ref coord) = self.coordinator {
                        if let Err(e) = coord.hangup(other_leg).await {
                            error!("Failed to terminate other leg {}: {}", other_leg, e);
                        }
                    }

                    // Clean up bridge
                    self.unregister_bridge(&bridge_id, &inbound, &outbound).await;
                    break;
                }

                LegEvent::OnHold { session_id } => {
                    // One leg on hold - hold the other
                    let other_leg = self.get_other_leg(&session_id).await;
                    if let Some(other) = other_leg {
                        info!("Bridge {}: Leg {} on hold, holding other leg {}",
                              bridge_id, session_id, other);

                        // Update state
                        if let Some(mut metadata) = self.bridge_metadata.get_mut(&bridge_id) {
                            metadata.state = BridgeState::OnHold;
                            metadata.stats.hold_resume_cycles += 1;
                        }

                        // Hold other leg
                        if let Some(ref coord) = self.coordinator {
                            if let Err(e) = coord.hold(&other).await {
                                warn!("Failed to hold other leg {}: {}", other, e);
                            }
                        }
                    }
                }

                LegEvent::Resumed { session_id } => {
                    // One leg resumed - resume the other
                    let other_leg = self.get_other_leg(&session_id).await;
                    if let Some(other) = other_leg {
                        info!("Bridge {}: Leg {} resumed, resuming other leg {}",
                              bridge_id, session_id, other);

                        // Update state
                        if let Some(mut metadata) = self.bridge_metadata.get_mut(&bridge_id) {
                            metadata.state = BridgeState::Active;
                        }

                        // Resume other leg
                        if let Some(ref coord) = self.coordinator {
                            if let Err(e) = coord.resume(&other).await {
                                warn!("Failed to resume other leg {}: {}", other, e);
                            }
                        }
                    }
                }

                LegEvent::DtmfReceived { session_id, digit } => {
                    // Forward DTMF to other leg
                    let other_leg = self.get_other_leg(&session_id).await;
                    if let Some(other) = other_leg {
                        debug!("Bridge {}: Forwarding DTMF '{}' from {} to {}",
                               bridge_id, digit, session_id, other);

                        // Update stats
                        if let Some(mut metadata) = self.bridge_metadata.get_mut(&bridge_id) {
                            metadata.stats.dtmf_forwarded += 1;
                        }

                        // Send DTMF to other leg
                        if let Some(ref coord) = self.coordinator {
                            if let Err(e) = coord.send_dtmf(&other, digit).await {
                                warn!("Failed to forward DTMF to {}: {}", other, e);
                            }
                        }
                    }
                }

                LegEvent::ReinviteReceived { session_id, sdp: _ } => {
                    // Handle re-INVITE by coordinating with other leg
                    let other_leg = self.get_other_leg(&session_id).await;
                    if let Some(other) = other_leg {
                        info!("Bridge {}: Re-INVITE from {}, coordinating with {}",
                              bridge_id, session_id, other);

                        // Update stats
                        if let Some(mut metadata) = self.bridge_metadata.get_mut(&bridge_id) {
                            metadata.stats.reinvites += 1;
                        }

                        // TODO: Handle SDP renegotiation between legs
                        // This would involve getting the new SDP offer, sending it to the
                        // other leg, getting the answer, and sending it back
                    }
                }

                LegEvent::MediaEvent { session_id, event } => {
                    match event {
                        MediaEventType::RtpTimeout => {
                            warn!("Bridge {}: RTP timeout on leg {}", bridge_id, session_id);

                            // Update stats
                            if let Some(mut metadata) = self.bridge_metadata.get_mut(&bridge_id) {
                                metadata.stats.media_interruptions += 1;
                            }

                            // Could terminate the call or try to recover
                        }
                        MediaEventType::PacketLoss { percentage } if percentage > 5.0 => {
                            warn!("Bridge {}: High packet loss ({}%) on leg {}",
                                  bridge_id, percentage, session_id);
                        }
                        _ => {
                            // Other media events
                            debug!("Bridge {}: Media event {:?} on leg {}",
                                   bridge_id, event, session_id);
                        }
                    }
                }

                LegEvent::StateChanged { session_id, new_state } => {
                    debug!("Bridge {}: Leg {} state changed to {:?}",
                           bridge_id, session_id, new_state);

                    // Update bridge state based on leg states
                    match new_state {
                        CallState::Ringing if session_id == outbound => {
                            if let Some(mut metadata) = self.bridge_metadata.get_mut(&bridge_id) {
                                metadata.state = BridgeState::Ringing;
                            }
                        }
                        CallState::Active => {
                            // Check if both legs are active
                            if let Some(ref coord) = self.coordinator {
                                let other_leg = self.get_other_leg(&session_id).await;
                                if let Some(other) = other_leg {
                                    if let Ok(other_state) = coord.get_state(&other).await {
                                        if other_state == CallState::Active {
                                            if let Some(mut metadata) =
                                                self.bridge_metadata.get_mut(&bridge_id) {
                                                metadata.state = BridgeState::Active;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }

                LegEvent::TransferRequested { session_id, target } => {
                    info!("Bridge {}: Transfer requested by {} to {}",
                          bridge_id, session_id, target);

                    // Handle transfer logic
                    // This would involve creating a new outbound leg to the target
                    // and replacing the existing bridge
                }
            }

            // Update duration periodically
            if let Some(mut metadata) = self.bridge_metadata.get_mut(&bridge_id) {
                metadata.stats.duration_secs = metadata.created_at.elapsed().as_secs();
            }
        }

        info!("Bridge {} coordination loop ended", bridge_id);
    }

    /// Get the other leg of a bridge
    async fn get_other_leg(&self, session_id: &SessionId) -> Option<SessionId> {
        if let Some(other) = self.bridges.get(session_id) {
            Some(other.clone())
        } else if let Some(other) = self.reverse_bridges.get(session_id) {
            Some(other.clone())
        } else {
            None
        }
    }

    /// Send event to a bridge
    pub async fn send_event(&self, session_id: &SessionId, event: LegEvent) -> Result<()> {
        // Find the bridge ID for this session
        if let Some(bridge_id) = self.session_to_bridge.get(session_id) {
            // Send event to the bridge's event channel
            if let Some(tx) = self.event_channels.get(bridge_id.value()) {
                tx.send(event).await
                    .map_err(|_| SessionError::Other("Bridge event channel closed".into()))?;
                Ok(())
            } else {
                Err(SessionError::Other(
                    format!("No event channel for bridge {}", bridge_id.value())
                ))
            }
        } else {
            Err(SessionError::Other(
                format!("Session {} is not part of a bridge", session_id)
            ))
        }
    }

    /// Clean up bridge
    async fn unregister_bridge(
        &self,
        bridge_id: &str,
        inbound: &SessionId,
        outbound: &SessionId,
    ) {
        // Update state
        if let Some(mut metadata) = self.bridge_metadata.get_mut(bridge_id) {
            metadata.state = BridgeState::Terminated;
        }

        // Remove mappings
        self.bridges.remove(inbound);
        self.reverse_bridges.remove(outbound);
        self.session_to_bridge.remove(inbound);
        self.session_to_bridge.remove(outbound);
        self.event_channels.remove(bridge_id);

        info!("Unregistered bridge {} between {} <-> {}", bridge_id, inbound, outbound);
    }

    /// Get bridge metadata
    pub fn get_bridge_metadata(&self, bridge_id: &str) -> Option<BridgeMetadata> {
        self.bridge_metadata.get(bridge_id).map(|m| m.clone())
    }

    /// Get bridge ID for a session
    pub fn get_bridge_id(&self, session_id: &SessionId) -> Option<String> {
        self.session_to_bridge.get(session_id).map(|id| id.clone())
    }

    /// List all active bridges
    pub fn list_bridges(&self) -> Vec<BridgeMetadata> {
        self.bridge_metadata.iter()
            .filter(|m| m.state != BridgeState::Terminated)
            .map(|m| m.clone())
            .collect()
    }

    /// Get statistics for a bridge
    pub fn get_bridge_stats(&self, bridge_id: &str) -> Option<BridgeStats> {
        self.bridge_metadata.get(bridge_id)
            .map(|m| m.stats.clone())
    }

    /// Terminate a bridge
    pub async fn terminate_bridge(&self, bridge_id: &str) -> Result<()> {
        // Find the sessions for this bridge
        let mut inbound_session = None;
        let mut outbound_session = None;

        for entry in self.session_to_bridge.iter() {
            if entry.value() == bridge_id {
                if self.bridges.contains_key(entry.key()) {
                    inbound_session = Some(entry.key().clone());
                } else if self.reverse_bridges.contains_key(entry.key()) {
                    outbound_session = Some(entry.key().clone());
                }
            }
        }

        if let (Some(inbound), Some(_outbound)) = (inbound_session, outbound_session) {
            // Send termination event
            self.send_event(
                &inbound,
                LegEvent::Terminated {
                    session_id: inbound.clone(),
                    reason: "Bridge terminated by request".to_string(),
                }
            ).await?;

            Ok(())
        } else {
            Err(SessionError::Other(
                format!("Bridge {} not found or incomplete", bridge_id)
            ))
        }
    }
}

impl Clone for BridgeCoordinator {
    fn clone(&self) -> Self {
        Self {
            bridges: self.bridges.clone(),
            reverse_bridges: self.reverse_bridges.clone(),
            event_channels: self.event_channels.clone(),
            bridge_metadata: self.bridge_metadata.clone(),
            session_to_bridge: self.session_to_bridge.clone(),
            coordinator: self.coordinator.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_bridge_registration() {
        let coordinator = BridgeCoordinator::new();

        let inbound = SessionId::new();
        let outbound = SessionId::new();

        let bridge_id = coordinator.register_bridge(
            inbound.clone(),
            outbound.clone(),
            MediaMode::Relay,
            "call-123".to_string(),
            "sip:alice@client.com".to_string(),
            "sip:service@server.com".to_string(),
            "sip:backend@internal.com".to_string(),
        ).await.unwrap();

        assert!(bridge_id.starts_with("bridge-"));

        // Verify mappings
        assert_eq!(coordinator.get_other_leg(&inbound).await, Some(outbound.clone()));
        assert_eq!(coordinator.get_other_leg(&outbound).await, Some(inbound.clone()));
        assert_eq!(coordinator.get_bridge_id(&inbound), Some(bridge_id.clone()));
        assert_eq!(coordinator.get_bridge_id(&outbound), Some(bridge_id));
    }

    #[tokio::test]
    async fn test_duplicate_bridge_prevention() {
        let coordinator = BridgeCoordinator::new();

        let inbound = SessionId::new();
        let outbound = SessionId::new();

        // First registration should succeed
        coordinator.register_bridge(
            inbound.clone(),
            outbound.clone(),
            MediaMode::Relay,
            "call-123".to_string(),
            "sip:alice@client.com".to_string(),
            "sip:service@server.com".to_string(),
            "sip:backend@internal.com".to_string(),
        ).await.unwrap();

        // Second registration with same inbound should fail
        let result = coordinator.register_bridge(
            inbound.clone(),
            SessionId::new(),
            MediaMode::Relay,
            "call-456".to_string(),
            "sip:bob@client.com".to_string(),
            "sip:service@server.com".to_string(),
            "sip:backend@internal.com".to_string(),
        ).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_event_sending() {
        let coordinator = BridgeCoordinator::new();

        let inbound = SessionId::new();
        let outbound = SessionId::new();

        let bridge_id = coordinator.register_bridge(
            inbound.clone(),
            outbound.clone(),
            MediaMode::Relay,
            "call-123".to_string(),
            "sip:alice@client.com".to_string(),
            "sip:service@server.com".to_string(),
            "sip:backend@internal.com".to_string(),
        ).await.unwrap();

        // Should be able to send event
        let result = coordinator.send_event(
            &inbound,
            LegEvent::DtmfReceived {
                session_id: inbound.clone(),
                digit: '5',
            }
        ).await;

        assert!(result.is_ok());
    }
}