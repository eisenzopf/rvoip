//! Gate 7 production-path SIP/RTP <-> WebRTC/RTP acceptance coverage.
//!
//! The bridge under test uses only the public `SipAdapter`, `WebRtcAdapter`,
//! and `Orchestrator` surfaces. The SIP peer is a wire fixture because the
//! point of the test is to observe the actual SIP and RTP boundary; the
//! WebRTC peer is another production rvoip adapter reached over WHIP or WS.
//!
//! Hermetic matrix:
//! - PCMU <-> Opus over WHIP
//! - PCMA <-> Opus over WebSocket signaling
//! - PCMU <-> Opus over authenticated WSS with an explicitly scoped test CA
//!
//! WHEP playback is covered separately because its media direction is
//! intentionally one-way. TURN and public-NAT variants require relay or
//! external network fixtures and remain in the broader deployment matrix.

#![cfg(all(
    feature = "tls-rustls",
    feature = "signaling-whip",
    feature = "signaling-ws"
))]

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use rcgen::generate_simple_self_signed;
use rvoip_core::adapter::{AdapterEvent, ConnectionAdapter, EndReason, OriginateRequest};
use rvoip_core::config::Config;
use rvoip_core::connection::{Direction, Transport};
use rvoip_core::conversation::ConversationPolicy;
use rvoip_core::events::Event;
use rvoip_core::ids::{ConnectionId, MessageId, ParticipantId, TenantId};
use rvoip_core::media_graph::{MediaGraphSnapshot, MediaGraphSourceState};
use rvoip_core::orchestrator::Orchestrator;
use rvoip_core::session::SessionMedium;
use rvoip_core::stream::{MediaFrame, MediaStream, StreamKind};
use rvoip_core::{DataMessage, DataReliability, DirectionalMediaBridgePlan};
use rvoip_sip::api::unified::{Config as SipConfig, UnifiedCoordinator};
use rvoip_sip::{SipAdapter, SipInitialHeaders, SipOriginateContext};
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::{Message as SipWireMessage, Method, Request};
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue, TypedHeader};
use rvoip_webrtc::signaling::auth::BearerStaticTokenAuth;
use rvoip_webrtc::signaling::whip::WhepServerMode;
use rvoip_webrtc::tls::TlsConfig;
use rvoip_webrtc::{
    StaticWebRtcBearerCredentialProvider, WebRtcAdapter, WebRtcBearerCredential, WebRtcConfig,
    WebRtcIceExchangePolicy, WebRtcOriginateContext, WebRtcServer, WebRtcServerBuilder,
    WebRtcSignalingMode, WebRtcTargetPolicy, WebRtcTlsClientTrust,
};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, watch};

const SIGNALING_TOKEN: &str = "sip-webrtc-acceptance";
const UAS_TAG: &str = "sip-webrtc-acceptance-uas";
const TELEPHONE_EVENT_PT: u8 = 101;
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug)]
enum SipCodec {
    Pcmu,
    Pcma,
}

impl SipCodec {
    const fn payload_type(self) -> u8 {
        match self {
            Self::Pcmu => 0,
            Self::Pcma => 8,
        }
    }

    const fn rtpmap(self) -> &'static str {
        match self {
            Self::Pcmu => "PCMU/8000",
            Self::Pcma => "PCMA/8000",
        }
    }

    const fn stream_name(self) -> &'static str {
        match self {
            Self::Pcmu => "g.711-mu",
            Self::Pcma => "g.711-a",
        }
    }

    const fn silence_octet(self) -> u8 {
        match self {
            Self::Pcmu => 0xff,
            Self::Pcma => 0xd5,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum SignalingCase {
    Whip,
    WebSocket,
    SecureWebSocket,
}

#[derive(Clone, Debug)]
struct EstablishedDialog {
    signaling_peer: SocketAddr,
    media_peer: SocketAddr,
    call_id: String,
    bridge_from: String,
    uas_to: String,
    initial_correlation_id: String,
    initial_account_tier: String,
}

#[derive(Debug)]
struct CapturedResponse {
    method: Method,
    status: u16,
}

struct SipWirePeer {
    signaling: Arc<UdpSocket>,
    media: Arc<UdpSocket>,
    // Keeps the SDP-declared tuple bound when the peer deliberately sends
    // from a different source port to exercise symmetric RTP learning.
    advertised_media: Option<Arc<UdpSocket>>,
    established: watch::Receiver<Option<EstablishedDialog>>,
    outbound_messages: mpsc::UnboundedReceiver<Request>,
    responses: mpsc::UnboundedReceiver<CapturedResponse>,
    bridge_teardowns: mpsc::UnboundedReceiver<Method>,
    task: tokio::task::JoinHandle<()>,
}

struct RemoteWebRtcServer {
    server: WebRtcServer,
    tls_trust: Option<Arc<WebRtcTlsClientTrust>>,
}

impl SipWirePeer {
    async fn start(codec: SipCodec, symmetric_rtp: bool) -> Self {
        let signaling = Arc::new(
            UdpSocket::bind("127.0.0.1:0")
                .await
                .expect("bind SIP acceptance UAS"),
        );
        let media = Arc::new(
            UdpSocket::bind("127.0.0.1:0")
                .await
                .expect("bind SIP acceptance RTP"),
        );
        let advertised_media = if symmetric_rtp {
            Some(Arc::new(
                UdpSocket::bind("127.0.0.1:0")
                    .await
                    .expect("bind SDP-declared SIP RTP tuple"),
            ))
        } else {
            None
        };
        let signaling_addr = signaling.local_addr().expect("SIP UAS address");
        let media_port = advertised_media.as_ref().map_or_else(
            || media.local_addr().expect("SIP media address").port(),
            |socket| {
                socket
                    .local_addr()
                    .expect("SDP-declared SIP media address")
                    .port()
            },
        );
        let (established_tx, established) = watch::channel(None);
        let (outbound_message_tx, outbound_messages) = mpsc::unbounded_channel();
        let (response_tx, responses) = mpsc::unbounded_channel();
        let (teardown_tx, bridge_teardowns) = mpsc::unbounded_channel();
        let task_socket = Arc::clone(&signaling);

        let task = tokio::spawn(async move {
            let mut packet = vec![0u8; 65_536];
            let mut bridge_media = None;
            let mut initial_context = None;
            loop {
                let Ok((bytes, peer)) = task_socket.recv_from(&mut packet).await else {
                    return;
                };
                let Ok(message) = parse_message(&packet[..bytes]) else {
                    continue;
                };
                match message {
                    SipWireMessage::Request(request) => match request.method() {
                        Method::Invite => {
                            bridge_media = parse_offer_media(&request, peer.ip());
                            initial_context = Some((
                                required_application_header(&request, "X-Correlation-Id"),
                                required_application_header(&request, "X-Account-Tier"),
                            ));
                            let sdp = format!(
                                "v=0\r\no=acceptance 1 1 IN IP4 127.0.0.1\r\n\
                                 s=sip-webrtc-acceptance\r\nc=IN IP4 127.0.0.1\r\n\
                                 t=0 0\r\nm=audio {media_port} RTP/AVP {} {TELEPHONE_EVENT_PT}\r\n\
                                 a=rtpmap:{} {}\r\n\
                                 a=rtpmap:{TELEPHONE_EVENT_PT} telephone-event/8000\r\n\
                                 a=fmtp:{TELEPHONE_EVENT_PT} 0-15\r\na=sendrecv\r\n",
                                codec.payload_type(),
                                codec.payload_type(),
                                codec.rtpmap(),
                            );
                            let response = success_response(
                                &request,
                                signaling_addr,
                                Some((&sdp, "application/sdp")),
                                true,
                            );
                            task_socket
                                .send_to(&response, peer)
                                .await
                                .expect("send SIP INVITE response");
                        }
                        Method::Ack => {
                            let Some(media_peer) = bridge_media else {
                                continue;
                            };
                            let Some((initial_correlation_id, initial_account_tier)) =
                                initial_context.clone()
                            else {
                                continue;
                            };
                            let dialog = EstablishedDialog {
                                signaling_peer: peer,
                                media_peer,
                                call_id: request
                                    .call_id()
                                    .map(|value| value.value())
                                    .expect("ACK Call-ID"),
                                bridge_from: required_header(&request, HeaderName::From),
                                uas_to: required_header(&request, HeaderName::To),
                                initial_correlation_id,
                                initial_account_tier,
                            };
                            established_tx.send_replace(Some(dialog));
                        }
                        Method::Message => {
                            outbound_message_tx
                                .send(request.clone())
                                .expect("outbound SIP MESSAGE receiver");
                            let response = success_response(&request, signaling_addr, None, false);
                            task_socket
                                .send_to(&response, peer)
                                .await
                                .expect("send SIP MESSAGE response");
                        }
                        Method::Bye | Method::Cancel => {
                            let _ = teardown_tx.send(request.method().clone());
                            let response = success_response(&request, signaling_addr, None, false);
                            task_socket
                                .send_to(&response, peer)
                                .await
                                .expect("send SIP teardown response");
                        }
                        _ => {}
                    },
                    SipWireMessage::Response(response) => {
                        let Some(cseq) = response.cseq() else {
                            continue;
                        };
                        let _ = response_tx.send(CapturedResponse {
                            method: cseq.method.clone(),
                            status: response.status().as_u16(),
                        });
                    }
                }
            }
        });

        Self {
            signaling,
            media,
            advertised_media,
            established,
            outbound_messages,
            responses,
            bridge_teardowns,
            task,
        }
    }

    fn address(&self) -> SocketAddr {
        self.signaling.local_addr().expect("SIP peer address")
    }

    async fn wait_established(&mut self) -> EstablishedDialog {
        tokio::time::timeout(TEST_TIMEOUT, async {
            loop {
                if let Some(dialog) = self.established.borrow_and_update().clone() {
                    return dialog;
                }
                self.established
                    .changed()
                    .await
                    .expect("SIP dialog watch closed");
            }
        })
        .await
        .expect("SIP dialog establishment deadline")
    }

    async fn send_audio_burst(&self, dialog: &EstablishedDialog, codec: SipCodec) {
        let payload = vec![codec.silence_octet(); 160];
        for sequence in 0..12u16 {
            let packet = rtp_packet(
                codec.payload_type(),
                sequence,
                u32::from(sequence) * 160,
                0x51_50_52_54,
                &payload,
            );
            self.media
                .send_to(&packet, dialog.media_peer)
                .await
                .expect("send SIP RTP audio");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn send_dtmf_burst(&self, dialog: &EstablishedDialog, event_code: u8) {
        let timestamp = 0x44_54_4d_46;
        let packets = [
            (80u16, [event_code, 10, 0, 0]),
            (81u16, [event_code, 10, 0, 160]),
            (82u16, [event_code, 0x80 | 10, 3, 32]),
            (83u16, [event_code, 0x80 | 10, 3, 32]),
            (84u16, [event_code, 0x80 | 10, 3, 32]),
        ];
        for (sequence, payload) in packets {
            let packet = rtp_packet(
                TELEPHONE_EVENT_PT,
                sequence,
                timestamp,
                0x44_54_4d_46,
                &payload,
            );
            self.media
                .send_to(&packet, dialog.media_peer)
                .await
                .expect("send SIP RFC 4733 RTP");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn receive_rtp_payload(&self, payload_type: u8) -> Vec<u8> {
        tokio::time::timeout(TEST_TIMEOUT, async {
            let mut packet = vec![0u8; 2_048];
            loop {
                let (bytes, _) = self
                    .media
                    .recv_from(&mut packet)
                    .await
                    .expect("receive RTP");
                if let Some((candidate, payload)) = parse_rtp(&packet[..bytes]) {
                    if candidate == payload_type {
                        return payload.to_vec();
                    }
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("RTP payload type {payload_type} deadline"))
    }

    async fn receive_dtmf_end(&self, event_code: u8) -> Vec<u8> {
        tokio::time::timeout(TEST_TIMEOUT, async {
            let mut packet = vec![0u8; 2_048];
            loop {
                let (bytes, _) = self
                    .media
                    .recv_from(&mut packet)
                    .await
                    .expect("receive DTMF RTP");
                let Some((payload_type, payload)) = parse_rtp(&packet[..bytes]) else {
                    continue;
                };
                if payload_type == TELEPHONE_EVENT_PT
                    && payload.len() >= 4
                    && payload[0] == event_code
                    && payload[1] & 0x80 != 0
                {
                    return payload.to_vec();
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("RFC 4733 end packet for event {event_code} deadline"))
    }

    async fn receive_outbound_message(&mut self) -> Request {
        tokio::time::timeout(TEST_TIMEOUT, self.outbound_messages.recv())
            .await
            .expect("outbound SIP MESSAGE deadline")
            .expect("outbound SIP MESSAGE channel")
    }

    async fn send_data_message(&mut self, dialog: &EstablishedDialog, message: &DataMessage) {
        let headers = format!(
            "MESSAGE sip:bridge@{} SIP/2.0\r\n\
             Via: SIP/2.0/UDP {};branch=z9hG4bK-context-{};rport\r\n\
             From: {}\r\nTo: {}\r\nCall-ID: {}\r\nCSeq: 2 MESSAGE\r\n\
             Max-Forwards: 70\r\nContact: <sip:acceptance@{}>\r\n\
             Content-Type: {}\r\nX-Bridgefu-Data-Label: {}\r\n\
             X-Bridgefu-Data-Content-Type: {}\r\nX-Bridgefu-Message-Id: {}\r\n\
             X-Bridgefu-Data-Reliability: reliable-ordered\r\nContent-Length: {}\r\n\r\n",
            dialog.signaling_peer,
            self.address(),
            message.message_id.as_str(),
            dialog.uas_to,
            dialog.bridge_from,
            dialog.call_id,
            self.address(),
            message.content_type,
            message.label,
            message.content_type,
            message.message_id.as_str(),
            message.bytes.len(),
        );
        let mut wire = headers.into_bytes();
        wire.extend_from_slice(&message.bytes);
        self.signaling
            .send_to(&wire, dialog.signaling_peer)
            .await
            .expect("send inbound SIP MESSAGE");
        self.expect_response(Method::Message).await;
    }

    async fn send_bye(&mut self, dialog: &EstablishedDialog) {
        let wire = format!(
            "BYE sip:bridge@{} SIP/2.0\r\n\
             Via: SIP/2.0/UDP {};branch=z9hG4bK-remote-bye;rport\r\n\
             From: {}\r\nTo: {}\r\nCall-ID: {}\r\nCSeq: 3 BYE\r\n\
             Max-Forwards: 70\r\nContent-Length: 0\r\n\r\n",
            dialog.signaling_peer,
            self.address(),
            dialog.uas_to,
            dialog.bridge_from,
            dialog.call_id,
        );
        self.signaling
            .send_to(wire.as_bytes(), dialog.signaling_peer)
            .await
            .expect("send remote SIP BYE");
        self.expect_response(Method::Bye).await;
    }

    async fn expect_response(&mut self, method: Method) {
        tokio::time::timeout(TEST_TIMEOUT, async {
            loop {
                let response = self.responses.recv().await.expect("SIP response channel");
                if response.method == method {
                    assert_eq!(response.status, 200, "{method:?} response status");
                    return;
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{method:?} response deadline"));
    }

    async fn assert_no_bridge_teardown(&mut self) {
        assert!(
            tokio::time::timeout(Duration::from_millis(50), self.bridge_teardowns.recv())
                .await
                .is_err(),
            "bridge initiated teardown before the remote BYE"
        );
    }

    async fn shutdown(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_sip_rtp_and_webrtc_rtp_bridge_acceptance() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();

    for (index, signaling, codec, symmetric_rtp) in [
        (0u16, SignalingCase::Whip, SipCodec::Pcmu, false),
        (1u16, SignalingCase::WebSocket, SipCodec::Pcma, false),
        (2u16, SignalingCase::SecureWebSocket, SipCodec::Pcmu, true),
    ] {
        run_case(index, signaling, codec, symmetric_rtp).await;
    }
    run_whep_playback_case(3, SipCodec::Pcma).await;
}

async fn run_case(index: u16, signaling: SignalingCase, codec: SipCodec, symmetric_rtp: bool) {
    let mut sip_peer = SipWirePeer::start(codec, symmetric_rtp).await;
    let mut server_config = WebRtcConfig::loopback();
    server_config.trickle_ice = true;
    let remote = build_remote_server(server_config, signaling).await;
    let server_adapter = remote.server.adapter();
    let mut server_events = server_adapter.subscribe_events();

    let sip_name = format!("sip-webrtc-{index}");
    let mut sip_config = SipConfig::local(&sip_name, 0);
    // One sequential test owns this small range. Keeping the cases distinct
    // also makes a leaked first case fail the second instead of cross-wiring.
    sip_config.media_port_start = 38_000 + index * 32;
    sip_config.media_port_end = sip_config.media_port_start + 31;
    let coordinator = UnifiedCoordinator::new(sip_config)
        .await
        .expect("SIP coordinator");
    let sip_adapter = SipAdapter::new(Arc::clone(&coordinator))
        .await
        .expect("SIP adapter");
    let webrtc_adapter = WebRtcAdapter::new(WebRtcConfig::loopback());

    let orchestrator = Orchestrator::new(Config::default());
    orchestrator
        .register(Arc::clone(&sip_adapter) as Arc<dyn ConnectionAdapter>)
        .expect("register production SIP adapter");
    orchestrator
        .register(Arc::clone(&webrtc_adapter) as Arc<dyn ConnectionAdapter>)
        .expect("register production WebRTC adapter");

    let conversation_id = orchestrator
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .expect("open acceptance conversation");
    let session_id = orchestrator
        .start_session(conversation_id.clone(), SessionMedium::Voice, vec![])
        .await
        .expect("start acceptance session");

    let sip_target = format!("sip:acceptance@{}", sip_peer.address());
    let expected_correlation_id = format!("acceptance-initial-{index}");
    let expected_account_tier = format!("tier-{index}");
    let initial_headers = SipInitialHeaders::new([
        ("X-Correlation-Id", expected_correlation_id.as_str()),
        ("X-Account-Tier", expected_account_tier.as_str()),
    ])
    .expect("allowlisted initial SIP headers");
    let sip_request = OriginateRequest::new(
        session_id.clone(),
        ParticipantId::new(),
        sip_target,
        Direction::Outbound,
        sip_adapter.capabilities(),
    )
    .with_transport(Transport::Sip)
    .with_context(SipOriginateContext::new().with_initial_headers(initial_headers));
    let sip_prepared = orchestrator
        .prepare_outbound_connection(sip_request)
        .await
        .expect("prepare production SIP route");
    let sip_handle = tokio::time::timeout(TEST_TIMEOUT, sip_prepared.commit())
        .await
        .expect("SIP commit deadline")
        .expect("commit production SIP route");
    let sip_connection = sip_handle.connection.id.clone();
    let dialog = sip_peer.wait_established().await;
    assert_eq!(dialog.initial_correlation_id, expected_correlation_id);
    assert_eq!(dialog.initial_account_tier, expected_account_tier);
    if symmetric_rtp {
        assert_ne!(
            sip_peer.media.local_addr().expect("SIP RTP source").port(),
            sip_peer
                .advertised_media
                .as_ref()
                .expect("declared symmetric-RTP target")
                .local_addr()
                .expect("declared symmetric-RTP address")
                .port(),
            "symmetric-RTP case must send from a tuple different from its SDP"
        );
    }

    let (endpoint, context) =
        outbound_webrtc_context(&remote.server, signaling, remote.tls_trust.clone());
    let webrtc_request = OriginateRequest::new(
        session_id.clone(),
        ParticipantId::new(),
        endpoint,
        Direction::Outbound,
        webrtc_adapter.capabilities(),
    )
    .with_transport(Transport::WebRtc)
    .with_context(context);
    let webrtc_prepared = orchestrator
        .prepare_outbound_connection(webrtc_request)
        .await
        .expect("prepare target-contacting WebRTC route");
    let webrtc_handle = tokio::time::timeout(TEST_TIMEOUT, webrtc_prepared.commit())
        .await
        .expect("WebRTC commit deadline")
        .expect("commit target-contacting WebRTC route");
    let webrtc_connection = webrtc_handle.connection.id.clone();
    let remote_connection = wait_for_inbound_connection(&mut server_events).await;

    tokio::time::timeout(TEST_TIMEOUT, async {
        let (client, remote) = tokio::join!(
            webrtc_adapter.accept(webrtc_connection.clone()),
            server_adapter.accept(remote_connection.clone()),
        );
        client.expect("bridge-side ICE/DTLS");
        remote.expect("remote ICE/DTLS");
    })
    .await
    .expect("WebRTC ICE/DTLS deadline");

    let sip_stream = wait_for_audio_codec(&sip_adapter, &sip_connection, codec.stream_name()).await;
    assert_eq!(sip_stream.codec().clock_rate_hz, 8_000);
    let remote_stream = audio_stream(server_adapter.as_ref(), &remote_connection).await;
    assert_eq!(remote_stream.codec().name.to_ascii_lowercase(), "opus");
    assert_eq!(remote_stream.codec().clock_rate_hz, 48_000);
    let mut remote_audio = remote_stream
        .try_frames_in()
        .expect("reserve remote WebRTC audio receiver");

    let bridge_id = orchestrator
        .bridge_connections(sip_connection.clone(), webrtc_connection.clone())
        .await
        .expect("bridge production SIP and WebRTC routes");
    let sip_graph = orchestrator
        .media_graph_for_connection(sip_connection.clone())
        .await
        .expect("SIP source MediaGraph");
    let webrtc_graph = orchestrator
        .media_graph_for_connection(webrtc_connection.clone())
        .await
        .expect("WebRTC source MediaGraph");

    // SIP/RTP G.711 -> graph transcode -> actual WebRTC/RTP Opus.
    sip_peer.send_audio_burst(&dialog, codec).await;
    let opus = match tokio::time::timeout(TEST_TIMEOUT, remote_audio.recv()).await {
        Ok(Some(frame)) => frame,
        Ok(None) => panic!(
            "remote WebRTC audio closed; SIP graph={:?}",
            sip_graph.snapshot().await
        ),
        Err(_) => panic!(
            "SIP-to-WebRTC audio deadline; dialog={dialog:?}; SIP graph={:?}",
            sip_graph.snapshot().await
        ),
    };
    assert_eq!(opus.payload_type, Some(111));
    assert!(!opus.payload.is_empty(), "transcoded Opus payload is empty");
    let sip_snapshot = wait_for_transcode(&sip_graph).await;
    assert_eq!(sip_snapshot.source_payload_type, codec.payload_type());
    assert_eq!(sip_snapshot.dropped_frames, 0);
    assert_eq!(sip_snapshot.evictions, 0);

    // Actual WebRTC/RTP Opus -> graph transcode -> SIP/RTP G.711.
    send_opus_burst(&remote_stream, 50).await;
    let g711 = sip_peer.receive_rtp_payload(codec.payload_type()).await;
    assert_eq!(g711.len(), 160, "G.711 must carry one 20 ms frame");
    let webrtc_snapshot = wait_for_transcode(&webrtc_graph).await;
    assert_eq!(webrtc_snapshot.source_payload_type, 111);
    assert_eq!(webrtc_snapshot.dropped_frames, 0);
    assert_eq!(webrtc_snapshot.evictions, 0);

    // WebRTC RFC 4733 is decoded to an AdapterEvent, automatically routed by
    // the live bridge, and emitted as a real SIP telephone-event RTP stream.
    server_adapter
        .send_dtmf(remote_connection.clone(), "5", 100)
        .await
        .expect("send WebRTC RFC 4733 DTMF");
    let telephone_event = sip_peer.receive_dtmf_end(5).await;
    assert!(telephone_event.len() >= 4, "short RFC 4733 payload");
    assert_eq!(telephone_event[0], 5, "RFC 4733 event code");
    assert_ne!(telephone_event[1] & 0x80, 0, "RFC 4733 end bit");
    assert!(u16::from_be_bytes([telephone_event[2], telephone_event[3]]) > 0);

    // The reverse path starts as real SIP PT 101 packets, is normalized to an
    // AdapterEvent by the SIP adapter, crosses the live bridge, and is emitted
    // and decoded as WebRTC RFC 4733 by the remote production adapter.
    sip_peer.send_dtmf_burst(&dialog, 6).await;
    let (digits, duration_ms) = wait_for_dtmf(&mut server_events, &remote_connection).await;
    assert_eq!(digits, "6");
    assert!(duration_ms > 0);

    // An arbitrary labeled binary DataChannel crosses the production bridge
    // and becomes an in-dialog SIP MESSAGE with an exact envelope.
    let arbitrary = DataMessage::try_new(
        "acceptance.arbitrary.binary",
        "application/octet-stream",
        Bytes::from_static(b"\0\xffwebrtc-to-sip\r\n"),
        DataReliability::ReliableOrdered,
        MessageId::from_string(format!("webrtc-to-sip-{index}")),
    )
    .expect("arbitrary DataMessage");
    server_adapter
        .send_data_message(remote_connection.clone(), arbitrary.clone())
        .await
        .expect("send arbitrary WebRTC DataChannel");
    let captured = sip_peer.receive_outbound_message().await;
    assert_eq!(captured.body, arbitrary.bytes);
    assert_eq!(
        application_header(&captured, "X-Bridgefu-Data-Label").as_deref(),
        Some(arbitrary.label.as_bytes())
    );
    assert_eq!(
        application_header(&captured, "X-Bridgefu-Message-Id").as_deref(),
        Some(arbitrary.message_id.as_str().as_bytes())
    );

    // The versioned Bridgefu context boundary works in the reverse direction:
    // later SIP context is a SIP MESSAGE and arrives on its exact DataChannel.
    let context_message = DataMessage::try_new(
        "bridgefu.context.v1",
        "application/json",
        Bytes::from(format!(r#"{{"correlation_id":"acceptance-{index}"}}"#)),
        DataReliability::ReliableOrdered,
        MessageId::from_string(format!("sip-to-webrtc-{index}")),
    )
    .expect("Bridgefu context DataMessage");
    sip_peer.send_data_message(&dialog, &context_message).await;
    assert_eq!(
        wait_for_data_message(&mut server_events, &remote_connection).await,
        context_message
    );

    sip_peer.assert_no_bridge_teardown().await;
    while remote_audio.try_recv().is_ok() {}

    // A remote SIP BYE is authoritative. It closes the SIP graph and removes
    // the bridge without forwarding any RTP submitted after teardown.
    sip_peer.send_bye(&dialog).await;
    wait_for_connection_end(&orchestrator, &sip_connection).await;
    let sip_terminal = tokio::time::timeout(TEST_TIMEOUT, sip_graph.wait_closed())
        .await
        .expect("SIP graph close deadline")
        .expect("SIP graph close");
    assert_ne!(sip_terminal, MediaGraphSourceState::Open);
    wait_for_no_active_bridge(&orchestrator).await;
    assert!(sip_graph.latest_snapshot().sinks.is_empty());
    assert!(webrtc_graph.latest_snapshot().sinks.is_empty());

    while remote_audio.try_recv().is_ok() {}
    sip_peer.send_audio_burst(&dialog, codec).await;
    match tokio::time::timeout(Duration::from_millis(350), remote_audio.recv()).await {
        Err(_) | Ok(None) => {}
        Ok(Some(frame)) => panic!(
            "post-BYE RTP crossed bridge {bridge_id}: {} bytes",
            frame.payload.len()
        ),
    }

    // The application owns peer compensation after one logical leg ends.
    orchestrator
        .end_connection(webrtc_connection.clone(), EndReason::Normal)
        .await
        .expect("end WebRTC peer after remote SIP BYE");
    wait_for_connection_end(&orchestrator, &webrtc_connection).await;
    let webrtc_terminal = tokio::time::timeout(TEST_TIMEOUT, webrtc_graph.wait_closed())
        .await
        .expect("WebRTC graph close deadline")
        .expect("WebRTC graph close");
    assert_ne!(webrtc_terminal, MediaGraphSourceState::Open);
    wait_until(TEST_TIMEOUT, || {
        !server_adapter.is_connection_live(&remote_connection)
    })
    .await;

    orchestrator
        .end_session(session_id, EndReason::Normal)
        .await
        .expect("end acceptance session");
    orchestrator
        .close_conversation(conversation_id, false)
        .await
        .expect("close acceptance conversation");
    orchestrator.drain_prepared_outbound_connections().await;
    orchestrator.drain_connection_lifecycle_tasks().await;
    assert_eq!(orchestrator.connection_lifecycle_task_count(), 0);

    assert!(
        webrtc_adapter
            .drain_outbound_signaling(Duration::from_secs(3))
            .await,
        "target-contacting WebRTC signaling did not drain"
    );
    sip_adapter.drain().await.expect("drain SIP adapter");
    coordinator
        .shutdown_gracefully(Some(Duration::from_secs(2)))
        .await
        .expect("shutdown SIP coordinator");
    remote.server.shutdown().await;

    assert_eq!(sip_adapter.retained_task_count(), 0);
    assert!(webrtc_adapter.routes().is_empty());
    assert!(server_adapter.routes().is_empty());
    assert_eq!(webrtc_adapter.outbound_signaling_task_count(), 0);
    assert_eq!(webrtc_adapter.outbound_ws_hub_task_count(), 0);
    assert_webrtc_clean(&webrtc_adapter);
    assert_webrtc_clean(&server_adapter);
    sip_peer.shutdown().await;
}

async fn run_whep_playback_case(index: u16, codec: SipCodec) {
    let mut sip_peer = SipWirePeer::start(codec, false).await;
    let mut server_config = WebRtcConfig::loopback();
    server_config.trickle_ice = true;
    let remote = build_remote_whep_server(server_config).await;
    let server_adapter = remote.server.adapter();
    let mut server_events = server_adapter.subscribe_events();

    let sip_name = format!("sip-whep-{index}");
    let mut sip_config = SipConfig::local(&sip_name, 0);
    sip_config.media_port_start = 38_000 + index * 32;
    sip_config.media_port_end = sip_config.media_port_start + 31;
    let coordinator = UnifiedCoordinator::new(sip_config)
        .await
        .expect("WHEP SIP coordinator");
    let sip_adapter = SipAdapter::new(Arc::clone(&coordinator))
        .await
        .expect("WHEP SIP adapter");
    let webrtc_adapter = WebRtcAdapter::new(WebRtcConfig::loopback());

    let orchestrator = Orchestrator::new(Config::default());
    orchestrator
        .register(Arc::clone(&sip_adapter) as Arc<dyn ConnectionAdapter>)
        .expect("register WHEP SIP adapter");
    orchestrator
        .register(Arc::clone(&webrtc_adapter) as Arc<dyn ConnectionAdapter>)
        .expect("register WHEP WebRTC adapter");

    let conversation_id = orchestrator
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .expect("open WHEP conversation");
    let session_id = orchestrator
        .start_session(conversation_id.clone(), SessionMedium::Voice, vec![])
        .await
        .expect("start WHEP session");

    let initial_headers = SipInitialHeaders::new([
        ("X-Correlation-Id", format!("whep-initial-{index}")),
        ("X-Account-Tier", "whep-playback".to_string()),
    ])
    .expect("WHEP initial SIP headers");
    let sip_request = OriginateRequest::new(
        session_id.clone(),
        ParticipantId::new(),
        format!("sip:acceptance@{}", sip_peer.address()),
        Direction::Outbound,
        sip_adapter.capabilities(),
    )
    .with_transport(Transport::Sip)
    .with_context(SipOriginateContext::new().with_initial_headers(initial_headers));
    let sip_connection = tokio::time::timeout(
        TEST_TIMEOUT,
        orchestrator
            .prepare_outbound_connection(sip_request)
            .await
            .expect("prepare WHEP SIP route")
            .commit(),
    )
    .await
    .expect("WHEP SIP commit deadline")
    .expect("commit WHEP SIP route")
    .connection
    .id;
    let dialog = sip_peer.wait_established().await;
    assert_eq!(
        dialog.initial_correlation_id,
        format!("whep-initial-{index}")
    );
    assert_eq!(dialog.initial_account_tier, "whep-playback");

    let (endpoint, context) = outbound_whep_context(
        &remote.server,
        remote.tls_trust.clone().expect("WHEP test trust"),
    );
    let whep_request = OriginateRequest::new(
        session_id.clone(),
        ParticipantId::new(),
        endpoint,
        Direction::Outbound,
        webrtc_adapter.capabilities(),
    )
    .with_transport(Transport::WebRtc)
    .with_context(context);
    let whep_connection = tokio::time::timeout(
        TEST_TIMEOUT,
        orchestrator
            .prepare_outbound_connection(whep_request)
            .await
            .expect("prepare target-contacting WHEP route")
            .commit(),
    )
    .await
    .expect("WHEP commit deadline")
    .expect("commit target-contacting WHEP route")
    .connection
    .id;
    let remote_connection = wait_for_inbound_connection(&mut server_events).await;
    tokio::time::timeout(TEST_TIMEOUT, async {
        let (client, origin) = tokio::join!(
            webrtc_adapter.accept(whep_connection.clone()),
            server_adapter.accept(remote_connection.clone()),
        );
        client.expect("WHEP client ICE/DTLS");
        origin.expect("WHEP origin ICE/DTLS");
    })
    .await
    .expect("WHEP ICE/DTLS deadline");

    let sip_stream = wait_for_audio_codec(&sip_adapter, &sip_connection, codec.stream_name()).await;
    let remote_stream = audio_stream(server_adapter.as_ref(), &remote_connection).await;
    assert_eq!(remote_stream.codec().name.to_ascii_lowercase(), "opus");
    let _bridge_id = orchestrator
        .bridge_connections_directional(
            sip_connection.clone(),
            whep_connection.clone(),
            DirectionalMediaBridgePlan::new(false, true).expect("WHEP playback direction"),
        )
        .await
        .expect("bridge WHEP playback to SIP");
    let whep_graph = orchestrator
        .media_graph_for_connection(whep_connection.clone())
        .await
        .expect("WHEP source MediaGraph");

    // WHEP draft-04 playback is origin -> subscriber. The disabled SIP source
    // remains reservable and therefore was not silently consumed by the bridge.
    drop(
        sip_stream
            .reserve_frames_in()
            .expect("disabled SIP source receiver remains available"),
    );
    send_opus_burst(&remote_stream, 90).await;
    let g711 = sip_peer.receive_rtp_payload(codec.payload_type()).await;
    assert_eq!(
        g711.len(),
        160,
        "WHEP playback must produce one 20 ms G.711 frame"
    );
    let snapshot = wait_for_transcode(&whep_graph).await;
    assert_eq!(snapshot.source_payload_type, 111);
    assert_eq!(snapshot.dropped_frames, 0);
    assert_eq!(snapshot.evictions, 0);

    sip_peer.send_bye(&dialog).await;
    wait_for_connection_end(&orchestrator, &sip_connection).await;
    wait_for_no_active_bridge(&orchestrator).await;
    assert!(whep_graph.latest_snapshot().sinks.is_empty());

    orchestrator
        .end_connection(whep_connection.clone(), EndReason::Normal)
        .await
        .expect("end WHEP resource");
    wait_for_connection_end(&orchestrator, &whep_connection).await;
    let terminal = tokio::time::timeout(TEST_TIMEOUT, whep_graph.wait_closed())
        .await
        .expect("WHEP graph close deadline")
        .expect("WHEP graph close");
    assert_ne!(terminal, MediaGraphSourceState::Open);
    wait_until(TEST_TIMEOUT, || {
        !server_adapter.is_connection_live(&remote_connection)
    })
    .await;

    orchestrator
        .end_session(session_id, EndReason::Normal)
        .await
        .expect("end WHEP session");
    orchestrator
        .close_conversation(conversation_id, false)
        .await
        .expect("close WHEP conversation");
    orchestrator.drain_prepared_outbound_connections().await;
    orchestrator.drain_connection_lifecycle_tasks().await;
    assert_eq!(orchestrator.connection_lifecycle_task_count(), 0);
    assert!(
        webrtc_adapter
            .drain_outbound_signaling(Duration::from_secs(3))
            .await,
        "WHEP signaling did not drain"
    );
    sip_adapter.drain().await.expect("drain WHEP SIP adapter");
    coordinator
        .shutdown_gracefully(Some(Duration::from_secs(2)))
        .await
        .expect("shutdown WHEP SIP coordinator");
    remote.server.shutdown().await;

    assert_eq!(sip_adapter.retained_task_count(), 0);
    assert!(webrtc_adapter.routes().is_empty());
    assert!(server_adapter.routes().is_empty());
    assert_eq!(webrtc_adapter.outbound_signaling_task_count(), 0);
    assert_webrtc_clean(&webrtc_adapter);
    assert_webrtc_clean(&server_adapter);
    sip_peer.shutdown().await;
}

async fn build_remote_server(config: WebRtcConfig, signaling: SignalingCase) -> RemoteWebRtcServer {
    match signaling {
        SignalingCase::Whip => RemoteWebRtcServer {
            server: WebRtcServerBuilder::new(config)
                .with_whip("127.0.0.1:0")
                .with_whip_auth(Arc::new(BearerStaticTokenAuth::new(SIGNALING_TOKEN)))
                .build()
                .await
                .expect("build WHIP acceptance server"),
            tls_trust: None,
        },
        SignalingCase::WebSocket => RemoteWebRtcServer {
            server: WebRtcServerBuilder::new(config)
                .with_ws("127.0.0.1:0")
                .with_ws_auth(Arc::new(BearerStaticTokenAuth::new(SIGNALING_TOKEN)))
                .build()
                .await
                .expect("build WS acceptance server"),
            tls_trust: None,
        },
        SignalingCase::SecureWebSocket => {
            let (tls, trust) = self_signed_tls().await;
            RemoteWebRtcServer {
                server: WebRtcServerBuilder::new(config)
                    .with_wss("127.0.0.1:0", tls)
                    .with_ws_auth(Arc::new(BearerStaticTokenAuth::new(SIGNALING_TOKEN)))
                    .build()
                    .await
                    .expect("build WSS acceptance server"),
                tls_trust: Some(trust),
            }
        }
    }
}

async fn build_remote_whep_server(config: WebRtcConfig) -> RemoteWebRtcServer {
    let (tls, trust) = self_signed_tls().await;
    RemoteWebRtcServer {
        server: WebRtcServerBuilder::new(config)
            .with_whips("127.0.0.1:0", tls)
            .with_whip_auth(Arc::new(BearerStaticTokenAuth::new(SIGNALING_TOKEN)))
            .with_whep_server_mode(WhepServerMode::Draft04)
            .build()
            .await
            .expect("build WHEP acceptance server"),
        tls_trust: Some(trust),
    }
}

async fn self_signed_tls() -> (TlsConfig, Arc<WebRtcTlsClientTrust>) {
    let certificate =
        generate_simple_self_signed(vec!["localhost".into()]).expect("test certificate");
    let cert_pem = certificate.cert.pem().into_bytes();
    let key_pem = certificate.signing_key.serialize_pem().into_bytes();
    let trust = Arc::new(WebRtcTlsClientTrust::from_pem(&cert_pem).expect("test client trust"));
    let tls = TlsConfig::from_pem_bytes(&cert_pem, &key_pem)
        .await
        .expect("test server TLS");
    (tls, trust)
}

fn outbound_webrtc_context(
    server: &WebRtcServer,
    signaling: SignalingCase,
    tls_trust: Option<Arc<WebRtcTlsClientTrust>>,
) -> (String, WebRtcOriginateContext) {
    let (endpoint, port) = match signaling {
        SignalingCase::Whip => {
            let address = server.whip_addr().expect("WHIP acceptance address");
            (format!("http://{address}/whip/acceptance"), address.port())
        }
        SignalingCase::WebSocket => {
            let address = server.ws_addr().expect("WS acceptance address");
            (format!("ws://{address}/signal"), address.port())
        }
        SignalingCase::SecureWebSocket => {
            let address = server.wss_addr().expect("WSS acceptance address");
            (
                format!("wss://localhost:{}/signal", address.port()),
                address.port(),
            )
        }
    };
    let policy = WebRtcTargetPolicy::default()
        .allow_port(port)
        .allow_insecure(true)
        .allow_loopback(true)
        .with_timeouts(Duration::from_secs(3), TEST_TIMEOUT)
        .expect("bounded target policy");
    let provider = Arc::new(StaticWebRtcBearerCredentialProvider::new(
        WebRtcBearerCredential::new(SIGNALING_TOKEN).expect("test bearer"),
    ));
    let context = match signaling {
        SignalingCase::Whip => WebRtcOriginateContext::new(
            &endpoint,
            WebRtcSignalingMode::Whip,
            WebRtcIceExchangePolicy::Trickle,
            policy,
            Some(provider),
        )
        .expect("WHIP originate context"),
        SignalingCase::WebSocket => WebRtcOriginateContext::websocket(&endpoint, policy)
            .expect("WS originate context")
            .with_bearer_provider(provider),
        SignalingCase::SecureWebSocket => WebRtcOriginateContext::websocket(&endpoint, policy)
            .expect("WSS originate context")
            .with_bearer_provider(provider),
    };
    let context = tls_trust.map_or(context.clone(), |trust| context.with_tls_trust(trust));
    (endpoint, context)
}

fn outbound_whep_context(
    server: &WebRtcServer,
    tls_trust: Arc<WebRtcTlsClientTrust>,
) -> (String, WebRtcOriginateContext) {
    let address = server.whips_addr().expect("WHEP acceptance address");
    let endpoint = format!("https://localhost:{}/whep/acceptance", address.port());
    let policy = WebRtcTargetPolicy::default()
        .allow_port(address.port())
        .allow_loopback(true)
        .with_timeouts(Duration::from_secs(3), TEST_TIMEOUT)
        .expect("bounded WHEP target policy");
    let provider = Arc::new(StaticWebRtcBearerCredentialProvider::new(
        WebRtcBearerCredential::new(SIGNALING_TOKEN).expect("WHEP bearer"),
    ));
    let context = WebRtcOriginateContext::new(
        &endpoint,
        WebRtcSignalingMode::Whep,
        WebRtcIceExchangePolicy::Trickle,
        policy,
        Some(provider),
    )
    .expect("WHEP originate context")
    .with_tls_trust(tls_trust);
    (endpoint, context)
}

async fn wait_for_inbound_connection(events: &mut mpsc::Receiver<AdapterEvent>) -> ConnectionId {
    tokio::time::timeout(TEST_TIMEOUT, async {
        loop {
            match events.recv().await {
                Some(AdapterEvent::InboundConnection { connection }) => return connection.id,
                Some(_) => {}
                None => panic!("remote WebRTC event stream closed"),
            }
        }
    })
    .await
    .expect("remote WebRTC inbound deadline")
}

async fn wait_for_data_message(
    events: &mut mpsc::Receiver<AdapterEvent>,
    connection: &ConnectionId,
) -> DataMessage {
    tokio::time::timeout(TEST_TIMEOUT, async {
        loop {
            match events.recv().await {
                Some(AdapterEvent::DataMessage {
                    connection_id,
                    message,
                }) if &connection_id == connection => return message,
                Some(_) => {}
                None => panic!("remote WebRTC event stream closed before DataMessage"),
            }
        }
    })
    .await
    .expect("remote WebRTC DataMessage deadline")
}

async fn wait_for_dtmf(
    events: &mut mpsc::Receiver<AdapterEvent>,
    connection: &ConnectionId,
) -> (String, u32) {
    tokio::time::timeout(TEST_TIMEOUT, async {
        loop {
            match events.recv().await {
                Some(AdapterEvent::Dtmf {
                    connection_id,
                    digits,
                    duration_ms,
                }) if &connection_id == connection => return (digits, duration_ms),
                Some(_) => {}
                None => panic!("remote WebRTC event stream closed before DTMF"),
            }
        }
    })
    .await
    .expect("remote WebRTC RFC 4733 deadline")
}

async fn wait_for_audio_codec(
    adapter: &Arc<SipAdapter>,
    connection: &ConnectionId,
    expected: &str,
) -> Arc<dyn MediaStream> {
    tokio::time::timeout(TEST_TIMEOUT, async {
        loop {
            if let Ok(streams) = adapter.streams(connection.clone()).await {
                if let Some(stream) = streams
                    .into_iter()
                    .find(|stream| stream.kind() == StreamKind::Audio)
                {
                    if stream.codec().name.eq_ignore_ascii_case(expected) {
                        return stream;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("SIP negotiated codec {expected} deadline"))
}

async fn audio_stream(adapter: &WebRtcAdapter, connection: &ConnectionId) -> Arc<dyn MediaStream> {
    adapter
        .streams(connection.clone())
        .await
        .expect("WebRTC media streams")
        .into_iter()
        .find(|stream| stream.kind() == StreamKind::Audio)
        .expect("WebRTC audio stream")
}

async fn send_opus_burst(stream: &Arc<dyn MediaStream>, sequence_base: u32) {
    let output = stream.frames_out();
    for sequence in 0..12u32 {
        output
            .send(MediaFrame {
                stream_id: stream.id(),
                kind: StreamKind::Audio,
                payload: rvoip_webrtc::media::silent_opus_payload(),
                timestamp_rtp: (sequence_base + sequence) * 960,
                captured_at: chrono::Utc::now(),
                payload_type: Some(111),
            })
            .await
            .expect("send remote Opus frame");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_for_transcode(graph: &rvoip_core::MediaGraphHandle) -> MediaGraphSnapshot {
    tokio::time::timeout(TEST_TIMEOUT, async {
        loop {
            let snapshot = graph.snapshot().await;
            if snapshot.source_frames > 0 && snapshot.transcode_operations > 0 {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("MediaGraph transcode diagnostics deadline")
}

async fn wait_for_connection_end(orchestrator: &Arc<Orchestrator>, connection: &ConnectionId) {
    wait_until(TEST_TIMEOUT, || match orchestrator.capacity_report() {
        Event::CapacityReport {
            active_connections, ..
        } => {
            if active_connections == 0 {
                return true;
            }
            orchestrator.session_of(connection).is_none()
        }
        _ => unreachable!("capacity_report returns CapacityReport"),
    })
    .await;
}

async fn wait_for_no_active_bridge(orchestrator: &Arc<Orchestrator>) {
    wait_until(TEST_TIMEOUT, || {
        matches!(
            orchestrator.capacity_report(),
            Event::CapacityReport {
                active_bridges: 0,
                ..
            }
        )
    })
    .await;
}

async fn wait_until(mut remaining: Duration, mut predicate: impl FnMut() -> bool) {
    while !predicate() {
        assert!(!remaining.is_zero(), "bounded acceptance wait expired");
        let sleep = remaining.min(Duration::from_millis(10));
        tokio::time::sleep(sleep).await;
        remaining = remaining.saturating_sub(sleep);
    }
}

fn assert_webrtc_clean(adapter: &WebRtcAdapter) {
    let metrics = adapter.metrics();
    assert_eq!(
        metrics.active_sessions, 0,
        "active WebRTC routes: {metrics:?}"
    );
    assert_eq!(
        metrics.active_http_resources, 0,
        "HTTP resources: {metrics:?}"
    );
    assert_eq!(metrics.http_resource_tasks, 0, "HTTP tasks: {metrics:?}");
    assert_eq!(metrics.peer_session_tasks, 0, "peer tasks: {metrics:?}");
    assert_eq!(metrics.media_tasks, 0, "media tasks: {metrics:?}");
    assert_eq!(
        metrics.inbound_ws_connection_tasks, 0,
        "inbound WS tasks: {metrics:?}"
    );
}

fn required_header(request: &Request, name: HeaderName) -> String {
    request
        .raw_header_value(&name)
        .unwrap_or_else(|| panic!("missing required SIP header {name:?}"))
}

fn success_response(
    request: &Request,
    uas_addr: SocketAddr,
    body: Option<(&str, &str)>,
    add_to_tag: bool,
) -> Vec<u8> {
    let mut to = required_header(request, HeaderName::To);
    if add_to_tag && !to.to_ascii_lowercase().contains(";tag=") {
        to.push_str(";tag=");
        to.push_str(UAS_TAG);
    }
    let (body, content_type) = body.unwrap_or(("", ""));
    let content_type_header = if content_type.is_empty() {
        String::new()
    } else {
        format!("Content-Type: {content_type}\r\n")
    };
    let head = format!(
        "SIP/2.0 200 OK\r\nVia: {}\r\nFrom: {}\r\nTo: {}\r\n\
         Call-ID: {}\r\nCSeq: {}\r\nContact: <sip:acceptance@{}>\r\n\
         {}Content-Length: {}\r\n\r\n",
        required_header(request, HeaderName::Via),
        required_header(request, HeaderName::From),
        to,
        required_header(request, HeaderName::CallId),
        required_header(request, HeaderName::CSeq),
        uas_addr,
        content_type_header,
        body.len(),
    );
    let mut wire = head.into_bytes();
    wire.extend_from_slice(body.as_bytes());
    wire
}

fn parse_offer_media(request: &Request, fallback_ip: IpAddr) -> Option<SocketAddr> {
    let body = std::str::from_utf8(&request.body).ok()?;
    let ip = body
        .lines()
        .find_map(|line| line.strip_prefix("c=IN IP4 "))
        .and_then(|value| IpAddr::from_str(value.trim()).ok())
        .unwrap_or(fallback_ip);
    let port = body
        .lines()
        .find_map(|line| line.strip_prefix("m=audio "))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some(SocketAddr::new(ip, port))
}

fn rtp_packet(
    payload_type: u8,
    sequence: u16,
    timestamp: u32,
    ssrc: u32,
    payload: &[u8],
) -> Vec<u8> {
    let mut packet = Vec::with_capacity(12 + payload.len());
    packet.extend_from_slice(&[0x80, payload_type & 0x7f]);
    packet.extend_from_slice(&sequence.to_be_bytes());
    packet.extend_from_slice(&timestamp.to_be_bytes());
    packet.extend_from_slice(&ssrc.to_be_bytes());
    packet.extend_from_slice(payload);
    packet
}

fn parse_rtp(packet: &[u8]) -> Option<(u8, &[u8])> {
    if packet.len() < 12 || packet[0] >> 6 != 2 {
        return None;
    }
    let csrc_count = usize::from(packet[0] & 0x0f);
    let mut header_len = 12usize.checked_add(csrc_count.checked_mul(4)?)?;
    if packet[0] & 0x10 != 0 {
        if packet.len() < header_len + 4 {
            return None;
        }
        let extension_words = u16::from_be_bytes([packet[header_len + 2], packet[header_len + 3]]);
        header_len = header_len.checked_add(4 + usize::from(extension_words) * 4)?;
    }
    if header_len > packet.len() {
        return None;
    }
    let mut payload_end = packet.len();
    if packet[0] & 0x20 != 0 {
        let padding = usize::from(*packet.last()?);
        if padding == 0 || padding > payload_end.saturating_sub(header_len) {
            return None;
        }
        payload_end -= padding;
    }
    Some((packet[1] & 0x7f, &packet[header_len..payload_end]))
}

fn application_header(request: &Request, name: &str) -> Option<Vec<u8>> {
    request.headers.iter().find_map(|header| match header {
        TypedHeader::Other(HeaderName::Other(candidate), HeaderValue::Raw(value))
            if candidate.eq_ignore_ascii_case(name) =>
        {
            Some(value.clone())
        }
        _ => None,
    })
}

fn required_application_header(request: &Request, name: &str) -> String {
    let value = application_header(request, name)
        .unwrap_or_else(|| panic!("missing required application header {name}"));
    String::from_utf8(value)
        .unwrap_or_else(|_| panic!("application header {name} was not valid UTF-8"))
}
