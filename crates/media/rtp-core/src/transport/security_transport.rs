//! Security-aware RTP transport wrapper
//!
//! This module provides a wrapper around the UDP transport that adds SRTP encryption/decryption.

use async_trait::async_trait;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, error, trace};

use crate::error::Error;
use crate::packet::rtcp::RtcpPacket;
use crate::packet::RtpPacket;
use crate::srtp::SrtpContext;
use crate::traits::RtpEvent;
use crate::transport::{RtpTransport, UdpRtpTransport};
use crate::Result;
use tokio::sync::broadcast;

/// Security-aware RTP transport that wraps UDP transport with SRTP
pub struct SecurityRtpTransport {
    /// Underlying UDP transport
    inner: Arc<UdpRtpTransport>,

    /// SRTP context for encryption/decryption
    srtp_context: Arc<RwLock<Option<SrtpContext>>>,

    /// Whether SRTP is enabled
    srtp_enabled: bool,

    /// Our own event broadcaster for decrypted events
    event_tx: broadcast::Sender<RtpEvent>,

    /// Tasks that intercept and authenticate muxed or separate RTP/RTCP.
    raw_packet_tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

impl SecurityRtpTransport {
    /// Create a new security-aware transport
    pub async fn new(inner: Arc<UdpRtpTransport>, srtp_enabled: bool) -> Result<Self> {
        // Create our own event broadcast channel
        let (event_tx, _) = broadcast::channel(100);

        // If SRTP is enabled, stop the inner transport's receiver to avoid conflicts
        if srtp_enabled {
            // Latch the inner transport before exposing it through
            // `inner_transport`, so its public raw-byte API cannot bypass this
            // wrapper's SRTP/SRTCP path.
            inner.require_srtp();
            debug!("Stopping inner UDP transport receiver to avoid socket conflicts");
            inner.stop_receiver().await?;
        }

        let transport = Self {
            inner,
            srtp_context: Arc::new(RwLock::new(None)),
            srtp_enabled,
            event_tx,
            raw_packet_tasks: Arc::new(Mutex::new(Vec::with_capacity(2))),
        };

        // Start the raw packet interception task if SRTP is enabled
        if srtp_enabled {
            transport.start_raw_packet_task().await?;
        }

        Ok(transport)
    }

    /// Start the raw packet interception task that processes packets before RTP parsing
    async fn start_raw_packet_task(&self) -> Result<()> {
        let mut tasks = vec![Self::spawn_raw_receiver(
            self.inner.get_socket(),
            self.srtp_context.clone(),
            self.event_tx.clone(),
            false,
        )];
        if let Some(rtcp_socket) = self.inner.get_rtcp_socket() {
            tasks.push(Self::spawn_raw_receiver(
                rtcp_socket,
                self.srtp_context.clone(),
                self.event_tx.clone(),
                true,
            ));
        }

        *self.raw_packet_tasks.lock().await = tasks;

        Ok(())
    }

    fn spawn_raw_receiver(
        socket: Arc<tokio::net::UdpSocket>,
        srtp_context: Arc<RwLock<Option<SrtpContext>>>,
        event_tx: broadcast::Sender<RtpEvent>,
        rtcp_only: bool,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut buffer = vec![0u8; crate::DEFAULT_MAX_PACKET_SIZE];
            loop {
                match socket.recv_from(&mut buffer).await {
                    Ok((size, addr)) => {
                        let packet_data = &buffer[..size];
                        let is_rtcp = super::udp::is_rtcp_packet(packet_data);
                        if rtcp_only && !is_rtcp {
                            trace!("Dropping non-RTCP datagram received on the secure RTCP socket");
                            continue;
                        }

                        let mut guard = srtp_context.write().await;
                        let Some(context) = guard.as_mut() else {
                            trace!("Dropping secure media because no cryptographic context is installed");
                            continue;
                        };

                        let event = if is_rtcp {
                            match context.unprotect_rtcp(packet_data) {
                                Ok(data) => RtpEvent::RtcpReceived { data, source: addr },
                                Err(error) => {
                                    trace!(
                                        "Dropping packet that failed SRTCP authentication: {error}"
                                    );
                                    continue;
                                }
                            }
                        } else {
                            match context.unprotect(packet_data) {
                                Ok(packet) => RtpEvent::MediaReceived {
                                    payload_type: packet.header.payload_type,
                                    sequence_number: packet.header.sequence_number,
                                    timestamp: packet.header.timestamp,
                                    marker: packet.header.marker,
                                    payload: packet.payload,
                                    padding_size: packet.padding_size,
                                    source: addr,
                                    ssrc: packet.header.ssrc,
                                },
                                Err(error) => {
                                    trace!(
                                        "Dropping packet that failed SRTP authentication: {error}"
                                    );
                                    continue;
                                }
                            }
                        };
                        drop(guard);
                        let _ = event_tx.send(event);
                    }
                    Err(error) => {
                        error!("Error receiving secure media packet: {error}");
                        let _ = event_tx.send(RtpEvent::Error(Error::Transport(format!(
                            "secure media socket error: {error}"
                        ))));
                        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                    }
                }
            }
        })
    }

    /// Set the bidirectional SRTP/SRTCP context for a secure wrapper.
    pub async fn set_srtp_context(&self, context: SrtpContext) -> Result<()> {
        if !self.srtp_enabled {
            return Err(Error::InvalidState(
                "cannot install an SRTP context on a plaintext security transport".to_string(),
            ));
        }
        context.validate_for_secure_transport()?;
        let mut srtp_guard = self.srtp_context.write().await;
        *srtp_guard = Some(context);
        debug!("SRTP context set on security transport");
        Ok(())
    }

    /// Get the underlying UDP transport for low-level diagnostics.
    ///
    /// Its raw socket handle is an unauthenticated escape from this wrapper.
    /// Public RTP byte sends remain latched closed in SRTP mode, but callers
    /// using `UdpRtpTransport::get_socket` directly bypass all SRTP policy and
    /// must not treat those reads or writes as protected media operations.
    pub fn inner_transport(&self) -> &Arc<UdpRtpTransport> {
        &self.inner
    }

    /// Check if SRTP is enabled and available
    pub async fn is_srtp_ready(&self) -> bool {
        if !self.srtp_enabled {
            return false;
        }
        let srtp_guard = self.srtp_context.read().await;
        srtp_guard
            .as_ref()
            .is_some_and(|context| context.validate_for_secure_transport().is_ok())
    }
}

#[async_trait]
impl RtpTransport for SecurityRtpTransport {
    async fn send_stun_bytes(&self, payload: &[u8], destination: SocketAddr) -> Result<()> {
        // ICE sits below SRTP: checks go out in plaintext even on a secured
        // transport. The inner send validates the payload really is STUN,
        // which is what keeps this from becoming a latch bypass.
        self.inner.send_stun_bytes(payload, destination).await
    }

    fn local_rtp_addr(&self) -> Result<SocketAddr> {
        self.inner.local_rtp_addr()
    }

    fn local_rtcp_addr(&self) -> Result<Option<SocketAddr>> {
        self.inner.local_rtcp_addr()
    }

    async fn send_rtp(&self, packet: &RtpPacket, dest: SocketAddr) -> Result<()> {
        if self.srtp_enabled {
            let mut srtp_guard = self.srtp_context.write().await;
            let context = srtp_guard.as_mut().ok_or_else(|| {
                Error::InvalidState("SRTP is enabled but no context is installed".to_string())
            })?;
            let protected = context.protect(packet)?.serialize()?;
            return self.inner.send_protected_rtp_bytes(&protected, dest).await;
        }
        self.inner.send_rtp(packet, dest).await
    }

    async fn send_rtp_bytes(&self, bytes: &[u8], dest: SocketAddr) -> Result<()> {
        if self.srtp_enabled {
            return Err(Error::InvalidState(
                "raw RTP bytes cannot bypass an enabled SRTP context".to_string(),
            ));
        }
        self.inner.send_rtp_bytes(bytes, dest).await
    }

    async fn send_rtcp(&self, packet: &RtcpPacket, dest: SocketAddr) -> Result<()> {
        self.send_rtcp_bytes(&packet.serialize()?, dest).await
    }

    async fn send_rtcp_bytes(&self, bytes: &[u8], dest: SocketAddr) -> Result<()> {
        if self.srtp_enabled {
            let mut srtp_guard = self.srtp_context.write().await;
            let context = srtp_guard.as_mut().ok_or_else(|| {
                Error::InvalidState("SRTCP is enabled but no context is installed".to_string())
            })?;
            let protected = context.protect_rtcp(bytes)?;
            return self.inner.send_protected_rtcp_bytes(&protected, dest).await;
        }
        self.inner.send_rtcp_bytes(bytes, dest).await
    }

    async fn receive_packet(&self, buffer: &mut [u8]) -> Result<(usize, SocketAddr)> {
        if self.srtp_enabled {
            return Err(Error::UnsupportedFeature(
                "direct receive is unavailable in secure mode; use authenticated events"
                    .to_string(),
            ));
        }
        self.inner.receive_packet(buffer).await
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn subscribe(&self) -> broadcast::Receiver<RtpEvent> {
        if self.srtp_enabled {
            self.event_tx.subscribe()
        } else {
            self.inner.subscribe()
        }
    }

    async fn close(&self) -> Result<()> {
        // Stop the raw packet interception task
        let mut task_guard = self.raw_packet_tasks.lock().await;
        for task in task_guard.drain(..) {
            task.abort();
        }

        // Close the inner transport
        self.inner.close().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::rtcp::RtcpReceiverReport;
    use crate::srtp::{SrtpCryptoKey, SRTP_AES128_CM_SHA1_80};
    use crate::transport::RtpTransportConfig;
    use std::time::Duration;

    fn context() -> SrtpContext {
        SrtpContext::new(
            SRTP_AES128_CM_SHA1_80,
            SrtpCryptoKey::new(vec![0x41; 16], vec![0x52; 14]),
        )
        .unwrap()
    }

    fn config(name: &str, rtcp_mux: bool) -> RtpTransportConfig {
        RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: (!rtcp_mux).then(|| "127.0.0.1:0".parse().unwrap()),
            symmetric_rtp: false,
            rtcp_mux,
            session_id: Some(format!("security-srtcp-{name}-{rtcp_mux}")),
            use_port_allocator: false,
            buffer_config: Default::default(),
        }
    }

    #[tokio::test]
    async fn wrapper_protects_and_unprotects_srtcp_on_muxed_and_separate_sockets() {
        for rtcp_mux in [true, false] {
            let inner_a = Arc::new(UdpRtpTransport::new(config("a", rtcp_mux)).await.unwrap());
            let inner_b = Arc::new(UdpRtpTransport::new(config("b", rtcp_mux)).await.unwrap());
            let transport_a = SecurityRtpTransport::new(inner_a, true).await.unwrap();
            let transport_b = SecurityRtpTransport::new(inner_b, true).await.unwrap();
            transport_a.set_srtp_context(context()).await.unwrap();
            transport_b.set_srtp_context(context()).await.unwrap();

            let destination = if rtcp_mux {
                transport_b.local_rtp_addr().unwrap()
            } else {
                transport_b
                    .inner
                    .get_rtcp_socket()
                    .unwrap()
                    .local_addr()
                    .unwrap()
            };
            let report = RtcpPacket::ReceiverReport(RtcpReceiverReport::new(0x1234_5678));
            let expected = report.serialize().unwrap();
            let mut events = transport_b.subscribe();

            let attacker = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
            attacker.send_to(&expected, destination).await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(100), events.recv())
                    .await
                    .is_err(),
                "plaintext RTCP must not escape the secure wrapper"
            );

            transport_a.send_rtcp(&report, destination).await.unwrap();

            match tokio::time::timeout(Duration::from_secs(1), events.recv()).await {
                Ok(Ok(RtpEvent::RtcpReceived { data, .. })) => assert_eq!(data, expected),
                other => panic!("expected secure wrapper RTCP event, got {other:?}"),
            }

            let media = RtpPacket::new_with_payload(
                96,
                7,
                1_120,
                0x8765_4321,
                bytes::Bytes::from_static(b"authenticated media"),
            );
            let media_destination = transport_b.local_rtp_addr().unwrap();
            let plain_media = media.serialize().unwrap();
            attacker
                .send_to(&plain_media, media_destination)
                .await
                .unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(100), events.recv())
                    .await
                    .is_err(),
                "plaintext RTP must not escape the secure wrapper"
            );
            transport_a
                .send_rtp(&media, media_destination)
                .await
                .unwrap();
            match tokio::time::timeout(Duration::from_secs(1), events.recv()).await {
                Ok(Ok(RtpEvent::MediaReceived {
                    sequence_number,
                    payload,
                    ..
                })) => {
                    assert_eq!(sequence_number, 7);
                    assert_eq!(payload, media.payload);
                }
                other => panic!("expected secure wrapper RTP event, got {other:?}"),
            }
            transport_a.close().await.unwrap();
            transport_b.close().await.unwrap();
        }
    }

    #[tokio::test]
    async fn wrapper_never_falls_back_to_plaintext_without_a_context() {
        let sink = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let inner = Arc::new(
            UdpRtpTransport::new(config("no-context", true))
                .await
                .unwrap(),
        );
        let transport = SecurityRtpTransport::new(inner, true).await.unwrap();
        let report = RtcpPacket::ReceiverReport(RtcpReceiverReport::new(0x1234_5678));

        assert!(matches!(
            transport
                .send_rtcp(&report, sink.local_addr().unwrap())
                .await,
            Err(Error::InvalidState(_))
        ));
        let mut wire = [0u8; 128];
        assert!(
            tokio::time::timeout(Duration::from_millis(100), sink.recv_from(&mut wire))
                .await
                .is_err()
        );
        transport.close().await.unwrap();
    }
}
