//! WebRTC Data Channel abstraction.
//!
//! A [`DataChannel`] represents a single logical channel multiplexed over
//! an SCTP association. Each channel is identified by a stream id and has
//! a human-readable label plus optional configuration for ordering and
//! reliability.

use bytes::Bytes;

/// Configuration for creating a data channel.
#[derive(Debug, Clone)]
pub struct DataChannelConfig {
    /// Human-readable label for the channel.
    pub label: String,
    /// Whether messages on this channel must be delivered in order.
    pub ordered: bool,
    /// Maximum number of retransmissions (partial reliability). `None`
    /// means fully reliable.
    pub max_retransmits: Option<u16>,
    /// Sub-protocol negotiated for this channel (may be empty).
    pub protocol: String,
}

impl Default for DataChannelConfig {
    fn default() -> Self {
        Self {
            label: String::new(),
            ordered: true,
            max_retransmits: None,
            protocol: String::new(),
        }
    }
}

/// Lifecycle state of a data channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataChannelState {
    /// Channel open is in progress.
    Connecting,
    /// Channel is ready for data transfer.
    Open,
    /// Channel is being closed.
    Closing,
    /// Channel is fully closed.
    Closed,
}

/// Events emitted by the data channel layer.
#[derive(Debug, Clone)]
pub enum DataChannelEvent {
    /// A channel has been opened.
    Open(u16),
    /// A message was received on a channel.
    Message(u16, Vec<u8>),
    /// A channel has been closed.
    Close(u16),
    /// An error occurred on a channel.
    Error(u16, String),
}

/// A single WebRTC data channel.
#[derive(Debug, Clone)]
pub struct DataChannel {
    /// SCTP stream identifier.
    id: u16,
    /// Human-readable label.
    label: String,
    /// Channel configuration.
    config: DataChannelConfig,
    /// Current state.
    state: DataChannelState,
}

impl DataChannel {
    /// Create a new data channel in the `Connecting` state.
    pub fn new(id: u16, config: DataChannelConfig) -> Self {
        let label = config.label.clone();
        Self {
            id,
            label,
            config,
            state: DataChannelState::Connecting,
        }
    }

    /// The SCTP stream id for this channel.
    pub fn id(&self) -> u16 {
        self.id
    }

    /// The human-readable label.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The channel configuration.
    pub fn config(&self) -> &DataChannelConfig {
        &self.config
    }

    /// Current lifecycle state.
    pub fn state(&self) -> DataChannelState {
        self.state
    }

    /// Transition the channel to the `Open` state.
    pub fn set_open(&mut self) {
        self.state = DataChannelState::Open;
    }

    /// Transition the channel to the `Closing` state.
    pub fn set_closing(&mut self) {
        self.state = DataChannelState::Closing;
    }

    /// Transition the channel to the `Closed` state.
    pub fn set_closed(&mut self) {
        self.state = DataChannelState::Closed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_data_channel_lifecycle() {
        let config = DataChannelConfig {
            label: "chat".to_string(),
            ordered: true,
            max_retransmits: None,
            protocol: String::new(),
        };
        let mut ch = DataChannel::new(0, config);
        assert_eq!(ch.id(), 0);
        assert_eq!(ch.label(), "chat");
        assert_eq!(ch.state(), DataChannelState::Connecting);

        ch.set_open();
        assert_eq!(ch.state(), DataChannelState::Open);

        ch.set_closing();
        assert_eq!(ch.state(), DataChannelState::Closing);

        ch.set_closed();
        assert_eq!(ch.state(), DataChannelState::Closed);
    }

    #[test]
    fn test_default_config() {
        let config = DataChannelConfig::default();
        assert!(config.ordered);
        assert!(config.max_retransmits.is_none());
        assert!(config.label.is_empty());
        assert!(config.protocol.is_empty());
    }

    #[test]
    fn test_data_channel_event_variants() {
        let open = DataChannelEvent::Open(0);
        let msg = DataChannelEvent::Message(1, vec![1, 2, 3]);
        let close = DataChannelEvent::Close(2);
        let err = DataChannelEvent::Error(3, "test error".to_string());

        // Ensure Debug is implemented (compilation check)
        let _ = format!("{:?}", open);
        let _ = format!("{:?}", msg);
        let _ = format!("{:?}", close);
        let _ = format!("{:?}", err);
    }
}
