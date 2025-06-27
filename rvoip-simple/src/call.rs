//! Call management for RVOIP Simple

use std::time::{Duration, SystemTime};
use tokio::sync::{broadcast, mpsc};
use tracing::{info, warn, debug};

use crate::{SimpleVoipError, CallEvent, CallState, CallDirection, CallQuality, MediaStats};

/// Represents an active call
#[derive(Debug)]
pub struct Call {
    /// Unique call identifier
    pub id: String,
    /// Remote party (SIP URI or phone number)
    pub remote_party: String,
    /// Current call state
    pub state: CallState,
    /// Call direction (incoming/outgoing)
    pub direction: CallDirection,
    /// Call start time (when answered)
    pub start_time: Option<SystemTime>,
    /// Media statistics
    pub media_stats: MediaStats,
    /// Event receiver for call events
    event_rx: Option<broadcast::Receiver<CallEvent>>,
    /// Internal event sender
    event_tx: broadcast::Sender<CallEvent>,
    /// Command sender for call control
    command_tx: mpsc::Sender<CallCommand>,
}

/// Internal commands for call control
#[derive(Debug)]
enum CallCommand {
    Answer,
    Reject,
    Hangup,
    Hold,
    Unhold,
    SendDtmf(char),
    Mute(bool),
}

impl Call {
    /// Create a new outgoing call
    pub fn new_outgoing(id: String, remote_party: String) -> Self {
        let (event_tx, _) = broadcast::channel(100);
        let (command_tx, _command_rx) = mpsc::channel(10);
        
        Self {
            id,
            remote_party,
            state: CallState::Initiating,
            direction: CallDirection::Outgoing,
            start_time: None,
            media_stats: MediaStats::default(),
            event_rx: None,
            event_tx,
            command_tx,
        }
    }

    /// Create a new incoming call
    pub fn new_incoming(id: String, remote_party: String) -> Self {
        let (event_tx, _) = broadcast::channel(100);
        let (command_tx, _command_rx) = mpsc::channel(10);
        
        Self {
            id,
            remote_party,
            state: CallState::Ringing,
            direction: CallDirection::Incoming,
            start_time: None,
            media_stats: MediaStats::default(),
            event_rx: None,
            event_tx,
            command_tx,
        }
    }

    /// Subscribe to call events
    pub fn subscribe_events(&mut self) -> broadcast::Receiver<CallEvent> {
        let rx = self.event_tx.subscribe();
        self.event_rx = Some(rx);
        self.event_tx.subscribe()
    }

    /// Get the next call event (async iterator pattern)
    pub async fn next_event(&mut self) -> Option<CallEvent> {
        if let Some(rx) = &mut self.event_rx {
            match rx.recv().await {
                Ok(event) => Some(event),
                Err(_) => None,
            }
        } else {
            // Auto-subscribe if not already subscribed
            let mut rx = self.event_tx.subscribe();
            match rx.recv().await {
                Ok(event) => {
                    self.event_rx = Some(rx);
                    Some(event)
                }
                Err(_) => None,
            }
        }
    }

    /// Answer an incoming call
    pub async fn answer(&mut self) -> Result<(), SimpleVoipError> {
        if self.direction != CallDirection::Incoming {
            return Err(SimpleVoipError::invalid_state("Cannot answer outgoing call"));
        }

        if self.state != CallState::Ringing {
            return Err(SimpleVoipError::invalid_state("Call is not ringing"));
        }

        info!("Answering call {}", self.id);
        
        // Send command to underlying implementation
        self.command_tx.send(CallCommand::Answer).await
            .map_err(|_| SimpleVoipError::call("Failed to send answer command"))?;

        // Update state
        self.state = CallState::Answered;
        self.start_time = Some(SystemTime::now());

        // Emit event
        let _ = self.event_tx.send(CallEvent::Answered);
        let _ = self.event_tx.send(CallEvent::StateChanged(self.id.clone(), self.state.clone()));

        Ok(())
    }

    /// Reject an incoming call
    pub async fn reject(&mut self) -> Result<(), SimpleVoipError> {
        if self.direction != CallDirection::Incoming {
            return Err(SimpleVoipError::invalid_state("Cannot reject outgoing call"));
        }

        if self.state != CallState::Ringing {
            return Err(SimpleVoipError::invalid_state("Call is not ringing"));
        }

        info!("Rejecting call {}", self.id);

        // Send command to underlying implementation
        self.command_tx.send(CallCommand::Reject).await
            .map_err(|_| SimpleVoipError::call("Failed to send reject command"))?;

        // Update state
        self.state = CallState::Ended;

        // Emit event
        let _ = self.event_tx.send(CallEvent::Ended);
        let _ = self.event_tx.send(CallEvent::StateChanged(self.id.clone(), self.state.clone()));

        Ok(())
    }

    /// Hang up the call
    pub async fn hangup(&mut self) -> Result<(), SimpleVoipError> {
        if matches!(self.state, CallState::Ended | CallState::Failed(_)) {
            return Err(SimpleVoipError::invalid_state("Call already ended"));
        }

        info!("Hanging up call {}", self.id);

        // Send command to underlying implementation
        self.command_tx.send(CallCommand::Hangup).await
            .map_err(|_| SimpleVoipError::call("Failed to send hangup command"))?;

        // Update state
        self.state = CallState::Ended;

        // Emit event
        let _ = self.event_tx.send(CallEvent::Ended);
        let _ = self.event_tx.send(CallEvent::StateChanged(self.id.clone(), self.state.clone()));

        Ok(())
    }

    /// Put the call on hold
    pub async fn hold(&mut self) -> Result<(), SimpleVoipError> {
        if self.state != CallState::Answered {
            return Err(SimpleVoipError::invalid_state("Call must be answered to put on hold"));
        }

        info!("Putting call {} on hold", self.id);

        // Send command to underlying implementation
        self.command_tx.send(CallCommand::Hold).await
            .map_err(|_| SimpleVoipError::call("Failed to send hold command"))?;

        // Update state
        self.state = CallState::OnHold;

        // Emit event
        let _ = self.event_tx.send(CallEvent::StateChanged(self.id.clone(), self.state.clone()));

        Ok(())
    }

    /// Resume the call from hold
    pub async fn unhold(&mut self) -> Result<(), SimpleVoipError> {
        if self.state != CallState::OnHold {
            return Err(SimpleVoipError::invalid_state("Call is not on hold"));
        }

        info!("Resuming call {} from hold", self.id);

        // Send command to underlying implementation
        self.command_tx.send(CallCommand::Unhold).await
            .map_err(|_| SimpleVoipError::call("Failed to send unhold command"))?;

        // Update state
        self.state = CallState::Answered;

        // Emit event
        let _ = self.event_tx.send(CallEvent::StateChanged(self.id.clone(), self.state.clone()));

        Ok(())
    }

    /// Send DTMF digit
    pub async fn send_dtmf(&self, digit: char) -> Result<(), SimpleVoipError> {
        if !matches!(self.state, CallState::Answered) {
            return Err(SimpleVoipError::invalid_state("Call must be answered to send DTMF"));
        }

        if !digit.is_ascii_digit() && !matches!(digit, '*' | '#' | 'A'..='D') {
            return Err(SimpleVoipError::configuration("Invalid DTMF digit"));
        }

        debug!("Sending DTMF digit '{}' on call {}", digit, self.id);

        // Send command to underlying implementation
        self.command_tx.send(CallCommand::SendDtmf(digit)).await
            .map_err(|_| SimpleVoipError::call("Failed to send DTMF command"))?;

        Ok(())
    }

    /// Send multiple DTMF digits
    pub async fn send_dtmf_string(&self, digits: &str) -> Result<(), SimpleVoipError> {
        for digit in digits.chars() {
            self.send_dtmf(digit).await?;
            // Small delay between digits
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }

    /// Mute or unmute the call
    pub async fn mute(&self, muted: bool) -> Result<(), SimpleVoipError> {
        if !matches!(self.state, CallState::Answered | CallState::OnHold) {
            return Err(SimpleVoipError::invalid_state("Call must be active to mute/unmute"));
        }

        info!("Setting mute to {} for call {}", muted, self.id);

        // Send command to underlying implementation
        self.command_tx.send(CallCommand::Mute(muted)).await
            .map_err(|_| SimpleVoipError::call("Failed to send mute command"))?;

        Ok(())
    }

    /// Get call duration
    pub fn duration(&self) -> Option<Duration> {
        self.start_time.map(|start| {
            SystemTime::now().duration_since(start).unwrap_or_default()
        })
    }

    /// Check if the call is active (answered and not ended)
    pub fn is_active(&self) -> bool {
        matches!(self.state, CallState::Answered | CallState::OnHold)
    }

    /// Check if the call has ended
    pub fn is_ended(&self) -> bool {
        matches!(self.state, CallState::Ended | CallState::Failed(_))
    }

    /// Get current call quality metrics
    pub fn quality(&self) -> Option<&CallQuality> {
        self.media_stats.quality.as_ref()
    }

    /// Update media statistics (called internally)
    pub(crate) fn update_media_stats(&mut self, stats: MediaStats) {
        self.media_stats = stats;
        
        if let Some(quality) = &self.media_stats.quality {
            let _ = self.event_tx.send(CallEvent::QualityChanged(self.id.clone(), quality.clone()));
        }
    }

    /// Update call state (called internally)
    pub(crate) fn update_state(&mut self, new_state: CallState) {
        if self.state != new_state {
            let old_state = self.state.clone();
            self.state = new_state.clone();
            
            info!("Call {} state changed: {:?} -> {:?}", self.id, old_state, new_state);
            
            // Update start time when call is answered
            if matches!(new_state, CallState::Answered) && self.start_time.is_none() {
                self.start_time = Some(SystemTime::now());
            }
            
            let _ = self.event_tx.send(CallEvent::StateChanged(self.id.clone(), new_state));
        }
    }

    /// Emit a DTMF received event (called internally)
    pub(crate) fn emit_dtmf_received(&self, digit: char) {
        let _ = self.event_tx.send(CallEvent::DtmfReceived(self.id.clone(), digit));
    }

    /// Emit media connected event (called internally)
    pub(crate) fn emit_media_connected(&self) {
        let _ = self.event_tx.send(CallEvent::MediaConnected(self.id.clone()));
    }

    /// Emit media disconnected event (called internally)
    pub(crate) fn emit_media_disconnected(&self) {
        let _ = self.event_tx.send(CallEvent::MediaDisconnected(self.id.clone()));
    }
}

impl Drop for Call {
    fn drop(&mut self) {
        if !self.is_ended() {
            warn!("Call {} dropped without proper hangup", self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_call_creation() {
        let outgoing = Call::new_outgoing("test-1".to_string(), "friend@domain.com".to_string());
        assert_eq!(outgoing.direction, CallDirection::Outgoing);
        assert_eq!(outgoing.state, CallState::Initiating);

        let incoming = Call::new_incoming("test-2".to_string(), "caller@domain.com".to_string());
        assert_eq!(incoming.direction, CallDirection::Incoming);
        assert_eq!(incoming.state, CallState::Ringing);
    }

    #[tokio::test]
    async fn test_call_answer() {
        let mut call = Call::new_incoming("test".to_string(), "caller@domain.com".to_string());
        
        let result = call.answer().await;
        assert!(result.is_ok());
        assert_eq!(call.state, CallState::Answered);
        assert!(call.start_time.is_some());
    }

    #[tokio::test]
    async fn test_call_reject() {
        let mut call = Call::new_incoming("test".to_string(), "caller@domain.com".to_string());
        
        let result = call.reject().await;
        assert!(result.is_ok());
        assert_eq!(call.state, CallState::Ended);
    }

    #[tokio::test]
    async fn test_invalid_operations() {
        let mut outgoing = Call::new_outgoing("test".to_string(), "target@domain.com".to_string());
        
        // Cannot answer outgoing call
        let result = outgoing.answer().await;
        assert!(result.is_err());
        
        // Cannot reject outgoing call
        let result = outgoing.reject().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_dtmf_validation() {
        let call = Call::new_outgoing("test".to_string(), "target@domain.com".to_string());
        
        // Valid digits
        assert!(call.send_dtmf('1').await.is_err()); // Call not answered
        
        // Invalid digit (call not answered first, but test validation)
        assert!(call.send_dtmf('X').await.is_err());
    }

    #[test]
    fn test_call_duration() {
        let mut call = Call::new_incoming("test".to_string(), "caller@domain.com".to_string());
        
        // No duration when not started
        assert!(call.duration().is_none());
        
        // Set start time manually for test
        call.start_time = Some(SystemTime::now() - Duration::from_secs(30));
        let duration = call.duration().unwrap();
        assert!(duration.as_secs() >= 30);
    }

    #[test]
    fn test_call_state_checks() {
        let answered_call = Call {
            id: "test".to_string(),
            remote_party: "test@domain.com".to_string(),
            state: CallState::Answered,
            direction: CallDirection::Outgoing,
            start_time: Some(SystemTime::now()),
            media_stats: MediaStats::default(),
            event_rx: None,
            event_tx: broadcast::channel(10).0,
            command_tx: mpsc::channel(10).0,
        };
        
        assert!(answered_call.is_active());
        assert!(!answered_call.is_ended());
        
        let ended_call = Call {
            state: CallState::Ended,
            ..answered_call
        };
        
        assert!(!ended_call.is_active());
        assert!(ended_call.is_ended());
    }
} 