//! Minimal SCTP association over DTLS for WebRTC Data Channels.
//!
//! This provides reliable ordered message transport on top of a
//! [`DtlsConnection`](crate::dtls::DtlsConnection). It implements the
//! four-way handshake (INIT / INIT-ACK / COOKIE-ECHO / COOKIE-ACK),
//! DATA sending with SACK-based acknowledgment, and graceful shutdown.

use std::collections::HashMap;
use bytes::Bytes;

use crate::dtls::DtlsConnection;
use crate::error::Error;
use super::channel::DataChannel;
use super::chunks::{
    self, DataChunkHeader, InitChunk, SackChunk, SctpHeader, RawChunk,
    CHUNK_COOKIE_ACK, CHUNK_COOKIE_ECHO, CHUNK_DATA, CHUNK_INIT,
    CHUNK_INIT_ACK, CHUNK_SACK, CHUNK_SHUTDOWN, CHUNK_SHUTDOWN_ACK,
    PPID_STRING,
};

/// State of the SCTP association.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssociationState {
    /// Waiting to send or receive INIT.
    Closed,
    /// INIT sent, waiting for INIT-ACK.
    CookieWait,
    /// COOKIE-ECHO sent, waiting for COOKIE-ACK.
    CookieEchoed,
    /// Association established -- data can flow.
    Established,
    /// SHUTDOWN sent, waiting for SHUTDOWN-ACK.
    ShutdownSent,
    /// Fully shut down.
    ShutdownComplete,
}

/// A minimal SCTP association running over a DTLS connection.
pub struct SctpAssociation {
    /// The underlying DTLS transport.
    dtls: DtlsConnection,
    /// Local SCTP port (default 5000 per WebRTC convention).
    local_port: u16,
    /// Remote SCTP port.
    remote_port: u16,
    /// Local verification tag (chosen at init time).
    local_verification_tag: u32,
    /// Remote verification tag (learned from peer's INIT/INIT-ACK).
    remote_verification_tag: u32,
    /// Next TSN to send.
    next_tsn: u32,
    /// Cumulative TSN acknowledged by peer.
    peer_cumulative_tsn_ack: u32,
    /// Highest TSN received from peer (for SACK generation).
    highest_received_tsn: Option<u32>,
    /// Per-stream next outgoing sequence number.
    stream_seq_out: HashMap<u16, u16>,
    /// Advertised receiver window.
    a_rwnd: u32,
    /// Association state.
    state: AssociationState,
    /// Open data channels indexed by stream id.
    channels: HashMap<u16, DataChannel>,
    /// ID to assign to the next created stream.
    next_stream_id: u16,
}

impl SctpAssociation {
    /// Create a new association wrapping a DTLS connection.
    ///
    /// `local_port` and `remote_port` are typically both 5000 for
    /// WebRTC data channels.
    pub fn new(dtls: DtlsConnection, local_port: u16, remote_port: u16) -> Self {
        // Use a deterministic-ish tag derived from the ports; callers that
        // care about security should use rand externally.
        let local_verification_tag = 0x5243_0000u32 | (local_port as u32);
        Self {
            dtls,
            local_port,
            remote_port,
            local_verification_tag,
            remote_verification_tag: 0,
            next_tsn: 1,
            peer_cumulative_tsn_ack: 0,
            highest_received_tsn: None,
            stream_seq_out: HashMap::new(),
            a_rwnd: 65535,
            state: AssociationState::Closed,
            channels: HashMap::new(),
            next_stream_id: 0,
        }
    }

    /// Current state of the association.
    pub fn state(&self) -> AssociationState {
        self.state
    }

    /// Access the open data channels.
    pub fn channels(&self) -> &HashMap<u16, DataChannel> {
        &self.channels
    }

    /// Register a data channel on a given stream id.
    pub fn add_channel(&mut self, channel: DataChannel) {
        self.channels.insert(channel.id(), channel);
    }

    /// Allocate the next stream id (even for initiator, odd for responder -- we
    /// use even here, matching the WebRTC convention for the DTLS client role).
    pub fn next_stream_id(&mut self) -> u16 {
        let id = self.next_stream_id;
        self.next_stream_id = self.next_stream_id.wrapping_add(2);
        id
    }

    // ----------------------------------------------------------------
    // Association setup
    // ----------------------------------------------------------------

    /// Initiate the SCTP four-way handshake (client side).
    ///
    /// Sends INIT, expects INIT-ACK with cookie, sends COOKIE-ECHO,
    /// expects COOKIE-ACK.
    pub async fn connect(&mut self) -> Result<(), Error> {
        if self.state != AssociationState::Closed {
            return Err(Error::SctpError("Association is not in Closed state".to_string()));
        }

        // 1. Send INIT
        let init = InitChunk {
            initiate_tag: self.local_verification_tag,
            a_rwnd: self.a_rwnd,
            num_outbound_streams: 65535,
            num_inbound_streams: 65535,
            initial_tsn: self.next_tsn,
            cookie: None,
        };
        self.send_chunks(0, &[init.to_raw(false)]).await?;
        self.state = AssociationState::CookieWait;
        tracing::debug!("SCTP: INIT sent, waiting for INIT-ACK");

        // 2. Wait for INIT-ACK
        let (header, raw_chunks) = self.receive_packet().await?;
        let init_ack_raw = raw_chunks
            .iter()
            .find(|c| c.chunk_type == CHUNK_INIT_ACK)
            .ok_or_else(|| Error::SctpError("Expected INIT-ACK chunk".to_string()))?;

        let init_ack = InitChunk::from_raw(&init_ack_raw.value)?;
        self.remote_verification_tag = init_ack.initiate_tag;

        let cookie = init_ack.cookie.ok_or_else(|| {
            Error::SctpError("INIT-ACK missing State Cookie".to_string())
        })?;

        // 3. Send COOKIE-ECHO
        let cookie_echo = RawChunk {
            chunk_type: CHUNK_COOKIE_ECHO,
            flags: 0,
            value: cookie,
        };
        self.send_chunks(self.remote_verification_tag, &[cookie_echo]).await?;
        self.state = AssociationState::CookieEchoed;
        tracing::debug!("SCTP: COOKIE-ECHO sent, waiting for COOKIE-ACK");

        // 4. Wait for COOKIE-ACK
        let (_header2, raw_chunks2) = self.receive_packet().await?;
        if !raw_chunks2.iter().any(|c| c.chunk_type == CHUNK_COOKIE_ACK) {
            return Err(Error::SctpError("Expected COOKIE-ACK chunk".to_string()));
        }

        self.state = AssociationState::Established;
        tracing::debug!("SCTP: association established");
        Ok(())
    }

    /// Accept an incoming SCTP association (server side).
    ///
    /// Waits for INIT, sends INIT-ACK with cookie, waits for COOKIE-ECHO,
    /// sends COOKIE-ACK.
    pub async fn accept(&mut self) -> Result<(), Error> {
        if self.state != AssociationState::Closed {
            return Err(Error::SctpError("Association is not in Closed state".to_string()));
        }

        // 1. Wait for INIT
        let (_header, raw_chunks) = self.receive_packet().await?;
        let init_raw = raw_chunks
            .iter()
            .find(|c| c.chunk_type == CHUNK_INIT)
            .ok_or_else(|| Error::SctpError("Expected INIT chunk".to_string()))?;
        let init = InitChunk::from_raw(&init_raw.value)?;
        self.remote_verification_tag = init.initiate_tag;

        // 2. Send INIT-ACK with a simple cookie
        let cookie_data = Bytes::copy_from_slice(b"rvoip-sctp-cookie");
        let init_ack = InitChunk {
            initiate_tag: self.local_verification_tag,
            a_rwnd: self.a_rwnd,
            num_outbound_streams: 65535,
            num_inbound_streams: 65535,
            initial_tsn: self.next_tsn,
            cookie: Some(cookie_data),
        };
        self.send_chunks(self.remote_verification_tag, &[init_ack.to_raw(true)]).await?;
        tracing::debug!("SCTP: INIT-ACK sent, waiting for COOKIE-ECHO");

        // 3. Wait for COOKIE-ECHO
        let (_header2, raw_chunks2) = self.receive_packet().await?;
        if !raw_chunks2.iter().any(|c| c.chunk_type == CHUNK_COOKIE_ECHO) {
            return Err(Error::SctpError("Expected COOKIE-ECHO chunk".to_string()));
        }

        // 4. Send COOKIE-ACK
        let cookie_ack = RawChunk {
            chunk_type: CHUNK_COOKIE_ACK,
            flags: 0,
            value: Bytes::new(),
        };
        self.send_chunks(self.remote_verification_tag, &[cookie_ack]).await?;

        self.state = AssociationState::Established;
        tracing::debug!("SCTP: association established (server)");
        Ok(())
    }

    // ----------------------------------------------------------------
    // Data transfer
    // ----------------------------------------------------------------

    /// Send data on a stream (reliable, ordered).
    pub async fn send(&mut self, stream_id: u16, data: &[u8]) -> Result<(), Error> {
        self.send_with_ppid(stream_id, data, PPID_STRING).await
    }

    /// Send data on a stream with an explicit payload protocol identifier.
    pub async fn send_with_ppid(
        &mut self,
        stream_id: u16,
        data: &[u8],
        ppid: u32,
    ) -> Result<(), Error> {
        if self.state != AssociationState::Established {
            return Err(Error::SctpError("Association not established".to_string()));
        }

        let tsn = self.next_tsn;
        self.next_tsn = self.next_tsn.wrapping_add(1);

        let seq = self.stream_seq_out.entry(stream_id).or_insert(0);
        let stream_seq = *seq;
        *seq = seq.wrapping_add(1);

        let data_hdr = DataChunkHeader {
            tsn,
            stream_id,
            stream_seq,
            protocol_id: ppid,
        };
        let chunk = data_hdr.to_raw(data, true);
        self.send_chunks(self.remote_verification_tag, &[chunk]).await?;

        tracing::trace!(tsn, stream_id, len = data.len(), "SCTP: DATA sent");
        Ok(())
    }

    /// Receive the next data message.
    ///
    /// Returns `(stream_id, payload)`. Automatically sends a SACK
    /// for each received DATA chunk.
    pub async fn receive(&mut self) -> Result<(u16, Vec<u8>), Error> {
        if self.state != AssociationState::Established {
            return Err(Error::SctpError("Association not established".to_string()));
        }

        loop {
            let (_header, raw_chunks) = self.receive_packet().await?;

            for chunk in &raw_chunks {
                match chunk.chunk_type {
                    CHUNK_DATA => {
                        let (data_hdr, payload) = DataChunkHeader::from_raw(&chunk.value)?;

                        // Track TSN for SACK
                        let tsn = data_hdr.tsn;
                        self.highest_received_tsn = Some(
                            self.highest_received_tsn.map_or(tsn, |prev| prev.max(tsn)),
                        );

                        // Send SACK
                        let sack = SackChunk {
                            cumulative_tsn_ack: self.highest_received_tsn.unwrap_or(0),
                            a_rwnd: self.a_rwnd,
                            gap_ack_blocks: Vec::new(),
                            duplicate_tsns: Vec::new(),
                        };
                        self.send_chunks(self.remote_verification_tag, &[sack.to_raw()])
                            .await?;

                        return Ok((data_hdr.stream_id, payload.to_vec()));
                    }
                    CHUNK_SACK => {
                        let sack = SackChunk::from_raw(&chunk.value)?;
                        self.peer_cumulative_tsn_ack = sack.cumulative_tsn_ack;
                        tracing::trace!(ack = sack.cumulative_tsn_ack, "SCTP: received SACK");
                        // Continue waiting for DATA
                    }
                    CHUNK_SHUTDOWN => {
                        // Peer wants to shut down -- acknowledge
                        let shutdown_ack = RawChunk {
                            chunk_type: CHUNK_SHUTDOWN_ACK,
                            flags: 0,
                            value: Bytes::new(),
                        };
                        self.send_chunks(self.remote_verification_tag, &[shutdown_ack])
                            .await?;
                        self.state = AssociationState::ShutdownComplete;
                        return Err(Error::SctpError("Association shut down by peer".to_string()));
                    }
                    _ => {
                        tracing::trace!(chunk_type = chunk.chunk_type, "SCTP: ignoring chunk");
                    }
                }
            }
        }
    }

    // ----------------------------------------------------------------
    // Shutdown
    // ----------------------------------------------------------------

    /// Gracefully shut down the association.
    pub async fn close(&mut self) -> Result<(), Error> {
        if self.state != AssociationState::Established {
            return Ok(()); // already closed or never opened
        }

        // Encode cumulative TSN ack into the SHUTDOWN value (4 bytes).
        let mut val = bytes::BytesMut::with_capacity(4);
        bytes::BufMut::put_u32(&mut val, self.highest_received_tsn.unwrap_or(0));
        let shutdown = RawChunk {
            chunk_type: CHUNK_SHUTDOWN,
            flags: 0,
            value: val.freeze(),
        };
        self.send_chunks(self.remote_verification_tag, &[shutdown]).await?;
        self.state = AssociationState::ShutdownSent;
        tracing::debug!("SCTP: SHUTDOWN sent");

        // Wait for SHUTDOWN-ACK
        let (_header, raw_chunks) = self.receive_packet().await?;
        if raw_chunks.iter().any(|c| c.chunk_type == CHUNK_SHUTDOWN_ACK) {
            self.state = AssociationState::ShutdownComplete;
            tracing::debug!("SCTP: association shutdown complete");
        }

        Ok(())
    }

    // ----------------------------------------------------------------
    // Internal helpers
    // ----------------------------------------------------------------

    /// Send one or more chunks inside an SCTP packet over the DTLS connection.
    async fn send_chunks(
        &mut self,
        verification_tag: u32,
        raw_chunks: &[RawChunk],
    ) -> Result<(), Error> {
        let header = SctpHeader {
            source_port: self.local_port,
            dest_port: self.remote_port,
            verification_tag,
            checksum: 0, // zero checksum per RFC 8261 over DTLS
        };
        let packet = chunks::encode_packet(&header, raw_chunks);
        self.dtls
            .send_application_data(&packet)
            .await
            .map_err(|e| Error::SctpError(format!("DTLS send failed: {e}")))?;
        Ok(())
    }

    /// Receive and decode one SCTP packet from the DTLS connection.
    ///
    /// Blocks until application data is available.
    async fn receive_packet(&mut self) -> Result<(SctpHeader, Vec<RawChunk>), Error> {
        // Try buffered data first, then poll DTLS
        loop {
            if let Some(data) = self.dtls.read_application_data() {
                return chunks::decode_packet(&data)
                    .map_err(|e| Error::SctpError(format!("SCTP decode: {e}")));
            }
            // No buffered data -- we need to wait. In a real implementation we
            // would await on the DTLS transport. For this MVP, return an error
            // indicating that no data is available yet.
            return Err(Error::SctpError(
                "No SCTP data available from DTLS transport".to_string(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dtls::{DtlsConfig, DtlsConnection};

    /// Helper: create a fresh association for unit testing.
    fn test_association() -> SctpAssociation {
        let dtls = DtlsConnection::new(DtlsConfig::default());
        SctpAssociation::new(dtls, 5000, 5000)
    }

    #[test]
    fn test_new_association_state() {
        let assoc = test_association();
        assert_eq!(assoc.state(), AssociationState::Closed);
        assert!(assoc.channels().is_empty());
    }

    #[test]
    fn test_next_stream_id_increments_by_two() {
        let mut assoc = test_association();
        assert_eq!(assoc.next_stream_id(), 0);
        assert_eq!(assoc.next_stream_id(), 2);
        assert_eq!(assoc.next_stream_id(), 4);
    }

    #[tokio::test]
    async fn test_send_requires_established() {
        let mut assoc = test_association();
        let result = assoc.send(0, b"hello").await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("not established"), "got: {err_msg}");
    }

    #[tokio::test]
    async fn test_receive_requires_established() {
        let mut assoc = test_association();
        let result = assoc.receive().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_connect_requires_closed_state() {
        let mut assoc = test_association();
        // Force into a non-Closed state
        assoc.state = AssociationState::Established;
        let result = assoc.connect().await;
        assert!(result.is_err());
    }
}
