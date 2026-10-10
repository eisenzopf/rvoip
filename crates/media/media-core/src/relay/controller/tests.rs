//! Unit tests for MediaSessionController
//!
//! This module contains all unit tests for the controller functionality.

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::types::{DialogId, MediaDirection, MediaSessionId};
    use bytes::Bytes;
    use rvoip_rtp_core::packet::RtpPacket;
    use rvoip_rtp_core::session::RtpSessionEvent;
    use rvoip_rtp_core::transport::{
        AllocationStrategy, PairingStrategy, PortAllocator, PortAllocatorConfig,
    };
    use std::collections::HashMap;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket as StdUdpSocket};
    use std::sync::Arc;

    #[cfg(feature = "dtls-srtp")]
    #[test]
    fn dtls_fingerprint_mismatch_fails_before_context_installation() {
        let advertised = [0x11; 32];
        assert!(verify_dtls_fingerprint(advertised, advertised).is_ok());

        let error = verify_dtls_fingerprint([0x22; 32], advertised)
            .expect_err("a certificate substitution must fail closed");
        assert!(matches!(error, Error::Config(_)));
    }

    #[tokio::test]
    async fn test_start_stop_session() {
        let controller = MediaSessionController::new();

        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: None,
            parameters: HashMap::new(),
        };

        // Start session
        let result = controller
            .start_media(DialogId::new("dialog1"), config)
            .await;
        assert!(result.is_ok());

        // Check session exists
        let session_info = controller.get_session_info(&DialogId::new("dialog1")).await;
        assert!(session_info.is_some());

        // Stop session
        let result = controller.stop_media(&DialogId::new("dialog1")).await;
        assert!(result.is_ok());

        // Check session is removed
        let session_info = controller.get_session_info(&DialogId::new("dialog1")).await;
        assert!(session_info.is_none());
    }

    #[tokio::test]
    async fn rtcp_xr_and_reduced_minimum_parameters_reach_the_rtp_session() {
        let controller = MediaSessionController::new();
        let dialog = DialogId::new("rtcp-policy");
        let base = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            remote_addr: None,
            preferred_codec: None,
            parameters: HashMap::new(),
        };
        // Defaults: neither is on, and an absent key means off.
        assert!(!base.rtcp_xr() && !base.rtcp_reduced_minimum());
        controller
            .start_media(
                dialog.clone(),
                base.clone()
                    .with_rtcp_xr(true)
                    .with_rtcp_reduced_minimum(true),
            )
            .await
            .unwrap();
        let session = controller.get_rtp_session(&dialog).await.unwrap();
        {
            let session = session.lock().await;
            assert!(session.rtcp_xr_enabled());
            assert!(session.rtcp_reduced_minimum());
        }

        let current = controller.get_session_info(&dialog).await.unwrap().config;
        controller
            .update_media(
                dialog.clone(),
                current.with_rtcp_xr(false).with_rtcp_reduced_minimum(false),
            )
            .await
            .unwrap();
        {
            let session = session.lock().await;
            assert!(!session.rtcp_xr_enabled());
            assert!(!session.rtcp_reduced_minimum());
        }
        let current = controller.get_session_info(&dialog).await.unwrap().config;
        assert!(!current.parameters.contains_key(RTCP_XR_PARAMETER));
        controller
            .update_media(dialog.clone(), current.with_rtcp_xr(true))
            .await
            .unwrap();
        assert!(session.lock().await.rtcp_xr_enabled());
        controller.stop_media(&dialog).await.unwrap();

        // A session created without the parameters keeps both off.
        let plain = DialogId::new("rtcp-policy-plain");
        controller.start_media(plain.clone(), base).await.unwrap();
        let session = controller.get_rtp_session(&plain).await.unwrap();
        let session = session.lock().await;
        assert!(!session.rtcp_xr_enabled());
        assert!(!session.rtcp_reduced_minimum());
    }

    fn loopback_config() -> MediaConfig {
        MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            remote_addr: None,
            preferred_codec: None,
            parameters: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn separate_rtcp_ports_are_paired_released_and_never_collide() {
        // An odd range start: pairs still begin on an even port.
        let controller = Arc::new(MediaSessionController::with_port_range(24_101, 24_180));
        let mut starts = Vec::new();
        for index in 0..30 {
            let controller = controller.clone();
            starts.push(tokio::spawn(async move {
                let dialog = DialogId::new(format!("pair-{index}"));
                // Every third call multiplexes and takes a single port.
                let separate = index % 3 != 0;
                controller
                    .start_media(
                        dialog.clone(),
                        loopback_config().with_rtcp_separate_port(separate),
                    )
                    .await
                    .expect("start media");
                let info = controller.get_session_info(&dialog).await.unwrap();
                (dialog, separate, info.rtp_port.unwrap(), info.rtcp_port)
            }));
        }
        let mut ports = std::collections::HashSet::new();
        let mut sessions = Vec::new();
        for start in starts {
            let (dialog, separate, rtp, rtcp) = start.await.unwrap();
            assert!(ports.insert(rtp), "RTP port {rtp} handed out twice");
            if separate {
                assert_eq!(rtp % 2, 0, "paired RTP port {rtp} is odd");
                assert_eq!(rtcp, Some(rtp + 1));
                assert!(
                    ports.insert(rtp + 1),
                    "RTCP port {} handed out twice",
                    rtp + 1
                );
            } else {
                assert_eq!(rtcp, None);
            }
            sessions.push((dialog, separate, rtcp));
        }
        assert_eq!(controller.allocated_port_count().await, 20 * 2 + 10);

        // Releasing the RTCP half after mux frees exactly that port, once.
        let (dialog, _, rtcp) = sessions
            .iter()
            .find(|(_, separate, _)| *separate)
            .cloned()
            .unwrap();
        assert!(controller.release_rtcp_port(&dialog).await.unwrap());
        assert!(!controller.release_rtcp_port(&dialog).await.unwrap());
        assert_eq!(
            controller
                .get_session_info(&dialog)
                .await
                .unwrap()
                .rtcp_port,
            None
        );
        std::net::UdpSocket::bind(("127.0.0.1", rtcp.unwrap())).expect("RTCP port is still bound");
        assert_eq!(controller.allocated_port_count().await, 20 * 2 + 10 - 1);

        for (dialog, _, _) in &sessions {
            controller.stop_media(dialog).await.unwrap();
        }
        assert_eq!(controller.allocated_port_count().await, 0, "ports leaked");
    }

    #[tokio::test]
    async fn separate_rtcp_bind_failure_moves_to_the_next_pair_and_leaks_nothing() {
        let controller = MediaSessionController::with_port_range(24_200, 24_203);
        // Someone else holds the first pair's RTCP port.
        let squatter = StdUdpSocket::bind(("127.0.0.1", 24_201)).unwrap();
        let dialog = DialogId::new("rtcp-bind-retry");
        controller
            .start_media(
                dialog.clone(),
                loopback_config().with_rtcp_separate_port(true),
            )
            .await
            .expect("the next pair is free");
        let info = controller.get_session_info(&dialog).await.unwrap();
        assert_eq!(
            (info.rtp_port, info.rtcp_port),
            (Some(24_202), Some(24_203))
        );
        controller.stop_media(&dialog).await.unwrap();
        assert_eq!(controller.allocated_port_count().await, 0);

        // With every pair blocked the start fails and holds no port.
        let squatter_two = StdUdpSocket::bind(("127.0.0.1", 24_203)).unwrap();
        let blocked = DialogId::new("rtcp-bind-blocked");
        controller
            .start_media(
                blocked.clone(),
                loopback_config().with_rtcp_separate_port(true),
            )
            .await
            .expect_err("no bindable pair");
        assert!(controller.get_session_info(&blocked).await.is_none());
        assert_eq!(
            controller.allocated_port_count().await,
            0,
            "failed start leaked"
        );
        drop((squatter, squatter_two));
    }

    #[tokio::test]
    async fn separate_rtcp_session_reports_to_the_signalled_rtcp_address() {
        let controller = MediaSessionController::with_port_range(24_300, 24_309);
        let dialog = DialogId::new("rtcp-signalled");
        controller
            .start_media(
                dialog.clone(),
                loopback_config().with_rtcp_separate_port(true),
            )
            .await
            .unwrap();
        let session = controller.get_rtp_session(&dialog).await.unwrap();
        {
            let mut session = session.lock().await;
            session.set_bandwidth(2_000_000);
            session.set_rtcp_reduced_minimum(true);
        }
        let peer_rtp = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let peer_rtcp = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let current = controller.get_session_info(&dialog).await.unwrap();
        let local_rtcp =
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), current.rtcp_port.unwrap());
        let mut config = current.config;
        config.remote_addr = Some(peer_rtp.local_addr().unwrap());
        controller
            .update_media(
                dialog.clone(),
                config.with_remote_rtcp_addr(Some(peer_rtcp.local_addr().unwrap())),
            )
            .await
            .unwrap();
        let mut bytes = [0u8; 2048];
        let (_, source) = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            peer_rtcp.recv_from(&mut bytes),
        )
        .await
        .expect("no RTCP at the a=rtcp: address")
        .unwrap();
        assert_eq!(source, local_rtcp);
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(600),
            peer_rtp.recv_from(&mut bytes)
        )
        .await
        .is_err());
        controller.stop_media(&dialog).await.unwrap();
    }

    #[tokio::test]
    async fn cancelled_start_releases_reserved_port_for_reuse() {
        let allocator = Arc::new(PortAllocator::with_config(PortAllocatorConfig {
            port_range_start: 15_500,
            port_range_end: 15_500,
            allocation_strategy: AllocationStrategy::Incremental,
            pairing_strategy: PairingStrategy::Muxed,
            prefer_port_reuse: false,
            default_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            allocation_retries: 1,
            validate_ports: false,
            capacity_hint: 1,
        }));
        let session_id = "cancelled-dialog".to_string();
        allocator
            .allocate_port_pair(&session_id, Some(IpAddr::V4(Ipv4Addr::LOCALHOST)))
            .await
            .expect("reserve sole port");
        assert_eq!(allocator.allocated_count().await, 1);

        // Dropping this armed guard is the exact path taken when the
        // start_media future is cancelled before map commit.
        drop(MediaPortReservationGuard::new(
            allocator.clone(),
            session_id,
        ));
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while allocator.allocated_count().await != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("cancellation cleanup timeout");

        allocator
            .allocate_port_pair("replacement-dialog", Some(IpAddr::V4(Ipv4Addr::LOCALHOST)))
            .await
            .expect("released sole port should be reusable");
    }

    #[tokio::test]
    async fn stop_media_clears_per_dialog_side_state_and_is_idempotent() {
        let controller = MediaSessionController::new();
        let dialog_id = DialogId::new("cleanup-dialog");
        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            remote_addr: None,
            preferred_codec: None,
            parameters: HashMap::new(),
        };

        controller
            .start_media(dialog_id.clone(), config)
            .await
            .expect("start media");

        let (audio_tx, _audio_rx) = tokio::sync::mpsc::channel(1);
        controller
            .set_audio_frame_callback(dialog_id.clone(), audio_tx)
            .await
            .expect("set audio callback");

        let (dtmf_tx, mut dtmf_rx) = tokio::sync::mpsc::channel(1);
        controller
            .set_dtmf_callback(dialog_id.clone(), dtmf_tx)
            .await
            .expect("set dtmf callback");

        controller.store_session_mapping(
            "session-cleanup".to_string(),
            MediaSessionId::from_dialog(&dialog_id),
        );
        controller
            .media_directions
            .insert(dialog_id.clone(), MediaDirection::SendRecv);

        let rtp_session = controller
            .rtp_sessions
            .get(&dialog_id)
            .expect("rtp session")
            .session
            .clone();
        let cn_gate = crate::relay::controller::cn_gate::CnGate::new(rtp_session).expect("cn gate");
        controller.cn_gate_state.insert(
            dialog_id.clone(),
            Arc::new(tokio::sync::Mutex::new(cn_gate)),
        );

        controller.stop_media(&dialog_id).await.expect("stop media");
        controller
            .stop_media(&dialog_id)
            .await
            .expect("second stop is idempotent");

        assert!(controller.sessions.is_empty());
        assert!(controller.rtp_sessions.is_empty());
        assert!(controller.audio_frame_callbacks.is_empty());
        assert!(controller.dtmf_callbacks.is_empty());
        assert!(controller.session_to_media.is_empty());
        assert!(controller.media_to_session.is_empty());
        assert!(controller.cn_gate_state.is_empty());
        assert!(controller.media_directions.is_empty());

        let closed = tokio::time::timeout(std::time::Duration::from_secs(1), dtmf_rx.recv())
            .await
            .expect("dtmf receiver should close");
        assert!(closed.is_none());
    }

    #[tokio::test]
    async fn stale_dialog_cleanup_cannot_remove_rebound_session_mapping() {
        let controller = MediaSessionController::new();
        let old_dialog = DialogId::new("rebound-old-dialog");
        let new_dialog = DialogId::new("rebound-new-dialog");
        let session_id = "reused-session".to_string();
        let old_media = MediaSessionId::from_dialog(&old_dialog);
        let new_media = MediaSessionId::from_dialog(&new_dialog);

        // Preserve the old reverse entry to model delayed cleanup while the
        // application-facing forward key has already been rebound.
        controller.store_session_mapping(session_id.clone(), old_media.clone());
        controller.store_session_mapping(session_id.clone(), new_media.clone());
        assert_eq!(
            controller.get_media_id(&session_id),
            Some(new_media.clone())
        );
        assert_eq!(
            controller.get_session_id(&old_media),
            Some(session_id.clone())
        );

        controller
            .stop_media(&old_dialog)
            .await
            .expect("stale stop remains idempotent");

        assert_eq!(
            controller.get_media_id(&session_id),
            Some(new_media.clone()),
            "old dialog cleanup must not remove the newer forward binding"
        );
        assert_eq!(controller.get_session_id(&new_media), Some(session_id));
        assert_eq!(controller.get_session_id(&old_media), None);
    }

    #[tokio::test]
    async fn decoded_audio_callback_preserves_rtp_timestamp() {
        let controller = MediaSessionController::new();
        let dialog_id = DialogId::new("rtp-timestamp-dialog");
        let (audio_tx, mut audio_rx) = tokio::sync::mpsc::channel(1);
        controller
            .set_audio_frame_callback(dialog_id.clone(), audio_tx)
            .await
            .expect("set audio callback");

        let codec = codec_runtime::resolve_codec(&MediaConfig {
            local_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            remote_addr: None,
            preferred_codec: Some("PCMU".to_string()),
            parameters: HashMap::new(),
        })
        .expect("resolve PCMU");
        controller.codec_runtimes.insert(
            dialog_id.clone(),
            Arc::new(codec_runtime::DialogCodecRuntime::new(codec).expect("create PCMU runtime")),
        );

        let (rtp_tx, rtp_rx) = tokio::sync::broadcast::channel(1);
        controller.spawn_rtp_event_handler(dialog_id, rtp_rx, 0);
        let timestamp = 0xf123_4567;
        rtp_tx
            .send(RtpSessionEvent::PacketReceived(
                RtpPacket::new_with_payload(
                    0,
                    7,
                    timestamp,
                    0x5256_4f49,
                    Bytes::from(vec![0xff; 160]),
                ),
            ))
            .expect("send RTP event");

        let frame = tokio::time::timeout(std::time::Duration::from_secs(1), audio_rx.recv())
            .await
            .expect("decoded frame timeout")
            .expect("decoded frame");
        assert_eq!(frame.timestamp, timestamp);
    }

    /// Exercise the actual RTP event handler, decode and callback, not just
    /// timestamp arithmetic. Both G.711 and the failing Opus path use it.
    #[cfg(feature = "opus")]
    #[tokio::test]
    async fn source_handoff_delivers_continuous_audible_frames() {
        use crate::codec::audio::common::AudioCodec;
        use crate::codec::audio::{G711Codec, OpusCodec, OpusConfig};
        use crate::types::SampleRate;

        for (opus, use_udp) in [(false, false), (true, false), (false, true), (true, true)] {
            let controller = MediaSessionController::new();
            let dialog = DialogId::new("source-handoff");
            let rate = if opus { 48_000 } else { 8_000 };
            let ticks = rate / 50;
            let pt = if opus { 102 } else { 0 };
            let mut parameters = HashMap::new();
            parameters.insert(types::RTP_PAYLOAD_TYPE_PARAMETER.into(), pt.to_string());
            parameters.insert(types::RTP_CLOCK_RATE_PARAMETER.into(), rate.to_string());
            parameters.insert(types::AUDIO_CHANNELS_PARAMETER.into(), "1".into());
            let udp = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let config = MediaConfig {
                local_addr: "127.0.0.1:0".parse().unwrap(),
                remote_addr: Some(udp.local_addr().unwrap()),
                preferred_codec: Some(if opus { "opus" } else { "PCMU" }.into()),
                parameters,
            };
            let (rtp_tx, rtp_rx) = tokio::sync::broadcast::channel(16);
            let mut udp_port = 0;
            if use_udp {
                controller
                    .start_media(dialog.clone(), config)
                    .await
                    .unwrap();
                udp_port = controller
                    .get_session_info(&dialog)
                    .await
                    .unwrap()
                    .rtp_port
                    .unwrap();
            } else {
                let format = codec_runtime::resolve_codec(&config).unwrap();
                controller.codec_runtimes.insert(
                    dialog.clone(),
                    Arc::new(codec_runtime::DialogCodecRuntime::new(format).unwrap()),
                );
                controller.spawn_rtp_event_handler(dialog.clone(), rtp_rx, pt);
            }
            let samples = (0..ticks)
                .map(|i| {
                    (8000.0 * (i as f64 * 440.0 * std::f64::consts::TAU / rate as f64).sin()) as i16
                })
                .collect();
            let frame = AudioFrame::new(samples, rate, 1, 0);
            let payload = if opus {
                OpusCodec::new(SampleRate::Rate48000, 1, OpusConfig::default())
                    .unwrap()
                    .encode(&frame)
                    .unwrap()
            } else {
                G711Codec::mu_law(rate, 1).unwrap().encode(&frame).unwrap()
            };
            let (audio_tx, mut audio_rx) = tokio::sync::mpsc::channel(16);
            controller
                .set_audio_frame_callback(dialog.clone(), audio_tx)
                .await
                .unwrap();
            // Independent clock origins and old-source packets after cutover.
            for (ssrc, seq, timestamp) in [
                (1, 10, 14_560),
                (1, 11, 14_560 + ticks),
                (2, 80, 456_000),
                (1, 12, 14_560 + 2 * ticks),
                (2, 81, 456_000 + ticks),
                (1, 13, 14_560 + 3 * ticks),
                (2, 82, 456_000 + 2 * ticks),
                (1, 14, 14_560 + 4 * ticks),
            ] {
                let packet = RtpPacket::new_with_payload(
                    pt,
                    seq,
                    timestamp,
                    ssrc,
                    Bytes::from(payload.clone()),
                );
                if use_udp {
                    udp.send_to(&packet.serialize().unwrap(), ("127.0.0.1", udp_port))
                        .await
                        .unwrap();
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                } else {
                    rtp_tx
                        .send(RtpSessionEvent::PacketReceived(packet))
                        .unwrap();
                }
            }
            drop(rtp_tx); // Handler drains events, then closes its callback clone.
            let mut previous = None;
            for _ in 0..5 {
                let frame =
                    tokio::time::timeout(std::time::Duration::from_secs(2), audio_rx.recv())
                        .await
                        .unwrap()
                        .unwrap();
                assert_eq!(frame.sample_rate, rate);
                assert_eq!(frame.samples.len(), ticks as usize);
                let energy = frame
                    .samples
                    .iter()
                    .map(|s| f64::from(*s).powi(2))
                    .sum::<f64>()
                    / frame.samples.len() as f64;
                assert!(energy > 100_000.0, "decoded speech must not become silence");
                if let Some(previous) = previous {
                    let step = frame.timestamp.wrapping_sub(previous);
                    assert!(
                        step >= ticks && step < rate / 5,
                        "artificial timestamp jump: {step}"
                    );
                }
                previous = Some(frame.timestamp);
            }
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(50), audio_rx.recv())
                    .await
                    .is_err(),
                "probation and retired-source packets must not reach the callback"
            );
            if use_udp {
                controller.stop_media(&dialog).await.unwrap();
            }
        }
    }

    /// Drive RTP packets through the real receive handler (reorder, source
    /// timeline, decode, paced playout, callback) and collect every frame
    /// that reaches the application callback.
    async fn receive_through_handler(
        codec: &str,
        parameters: HashMap<String, String>,
        payload_type: u8,
        packets: Vec<(u32, u16, u32, Vec<u8>)>,
    ) -> Vec<AudioFrame> {
        let controller = MediaSessionController::new();
        let dialog = DialogId::new("receive-through-handler");
        let format = codec_runtime::resolve_codec(&MediaConfig {
            local_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            remote_addr: None,
            preferred_codec: Some(codec.to_string()),
            parameters,
        })
        .expect("resolve codec");
        controller.codec_runtimes.insert(
            dialog.clone(),
            Arc::new(codec_runtime::DialogCodecRuntime::new(format).expect("codec runtime")),
        );
        let (audio_tx, mut audio_rx) = tokio::sync::mpsc::channel(64);
        controller
            .set_audio_frame_callback(dialog.clone(), audio_tx)
            .await
            .expect("set audio callback");
        let (rtp_tx, rtp_rx) = tokio::sync::broadcast::channel(64);
        controller.spawn_rtp_event_handler(dialog.clone(), rtp_rx, payload_type);
        for (ssrc, sequence, timestamp, payload) in packets {
            rtp_tx
                .send(RtpSessionEvent::PacketReceived(
                    RtpPacket::new_with_payload(
                        payload_type,
                        sequence,
                        timestamp,
                        ssrc,
                        Bytes::from(payload),
                    ),
                ))
                .expect("send RTP event");
        }
        let mut frames = Vec::new();
        while let Ok(Some(frame)) =
            tokio::time::timeout(std::time::Duration::from_millis(300), audio_rx.recv()).await
        {
            frames.push(frame);
        }
        drop(rtp_tx);
        frames
    }

    /// Regression: a sender that keeps its SSRC and sequence continuity but
    /// resets its RTP clock must not lose audio until the old clock catches
    /// up. Every packet is played and the delivered timeline stays continuous.
    #[tokio::test]
    async fn same_ssrc_timestamp_reset_keeps_audio_flowing() {
        let mut packets = Vec::new();
        for k in 0..5u16 {
            packets.push((
                0x1111,
                10 + k,
                3_000_000 + u32::from(k) * 160,
                vec![0x90; 160],
            ));
        }
        // Same SSRC, sequence continues, timestamp restarts near zero.
        for k in 0..10u16 {
            packets.push((0x1111, 15 + k, 320 + u32::from(k) * 160, vec![0x90; 160]));
        }
        let frames = receive_through_handler("PCMU", HashMap::new(), 0, packets).await;
        let timestamps: Vec<u32> = frames.iter().map(|frame| frame.timestamp).collect();
        assert_eq!(
            frames.len(),
            15,
            "every packet after the clock reset must be played: {timestamps:?}"
        );
        for pair in timestamps.windows(2) {
            assert_eq!(
                pair[1].wrapping_sub(pair[0]),
                160,
                "delivered timeline must stay continuous: {timestamps:?}"
            );
        }
    }

    /// The intended improvement: a direct peer replaces its SSRC with an
    /// independent RTP clock and an independent Opus encoder. Returns the
    /// frames delivered to the application callback, how many came from the
    /// first source, and a fresh decoder's output for each new-source packet.
    #[cfg(feature = "opus")]
    async fn opus_ssrc_replacement() -> (Vec<AudioFrame>, usize, Vec<Vec<i16>>) {
        let mut parameters = HashMap::new();
        parameters.insert(types::RTP_PAYLOAD_TYPE_PARAMETER.into(), "111".into());
        parameters.insert(types::RTP_CLOCK_RATE_PARAMETER.into(), "48000".into());
        parameters.insert(types::AUDIO_CHANNELS_PARAMETER.into(), "1".into());
        let format = codec_runtime::resolve_codec(&MediaConfig {
            local_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            remote_addr: None,
            preferred_codec: Some("opus".to_string()),
            parameters: parameters.clone(),
        })
        .expect("resolve opus");
        let tone = |hz: f64, frame: u32| {
            let samples = (0..960u32)
                .flat_map(|i| {
                    let n = f64::from(frame * 960 + i);
                    let sample =
                        (8000.0 * (n * hz * std::f64::consts::TAU / 48_000.0).sin()) as i16;
                    vec![sample; usize::from(format.channels)]
                })
                .collect();
            AudioFrame::new(samples, 48_000, format.channels, 0)
        };
        // Two independent senders, each with its own encoder.
        let sender_a = codec_runtime::DialogCodecRuntime::new(format.clone()).unwrap();
        let sender_b = codec_runtime::DialogCodecRuntime::new(format.clone()).unwrap();
        let mut a = Vec::new();
        for k in 0..6 {
            a.push(sender_a.encode(&tone(440.0, k)).await.unwrap());
        }
        let mut b = Vec::new();
        for k in 0..4 {
            b.push(sender_b.encode(&tone(880.0, k)).await.unwrap());
        }
        let mut packets = Vec::new();
        for (k, payload) in a.iter().enumerate() {
            packets.push((
                0xaaaa,
                10 + k as u16,
                14_560 + k as u32 * 960,
                payload.clone(),
            ));
        }
        for (k, payload) in b.iter().enumerate() {
            packets.push((
                0xbbbb,
                80 + k as u16,
                456_000 + k as u32 * 960,
                payload.clone(),
            ));
        }
        let frames = receive_through_handler("opus", parameters, 111, packets).await;
        let mut fresh = Vec::new();
        for payload in &b {
            let decoder = codec_runtime::DialogCodecRuntime::new(format.clone()).unwrap();
            fresh.push(decoder.decode(payload, 0).await.unwrap().samples);
        }
        (frames, a.len(), fresh)
    }

    /// The callback sees a continuous timeline, not the new source's raw epoch.
    #[cfg(feature = "opus")]
    #[tokio::test]
    async fn opus_ssrc_replacement_keeps_a_continuous_timeline() {
        let (frames, first_source_frames, _) = opus_ssrc_replacement().await;
        let timestamps: Vec<u32> = frames.iter().map(|frame| frame.timestamp).collect();
        assert!(
            frames.len() > first_source_frames,
            "replacement source must be played: {timestamps:?}"
        );
        for pair in timestamps.windows(2) {
            assert_eq!(
                pair[1].wrapping_sub(pair[0]),
                960,
                "artificial timestamp jump passed downstream: {timestamps:?}"
            );
        }
    }

    /// The first new-source frame the callback sees is what a fresh decoder
    /// produces for a new-source packet, not the output of a decoder still
    /// carrying the previous source's prediction and overlap state.
    #[cfg(feature = "opus")]
    #[tokio::test]
    async fn opus_ssrc_replacement_is_decoded_with_fresh_state() {
        let (frames, first_source_frames, fresh) = opus_ssrc_replacement().await;
        let first_new = &frames
            .get(first_source_frames)
            .expect("replacement source must be played")
            .samples;
        assert!(
            fresh.iter().any(|samples| samples == first_new),
            "replacement source was decoded with the previous source's decoder state"
        );
    }

    #[tokio::test]
    async fn test_dynamic_port_allocation() {
        println!("🧪 Testing dynamic port allocation integration");

        let controller = MediaSessionController::new();

        // Create multiple sessions to verify different ports are allocated
        let mut session_infos = Vec::new();

        for i in 0..3 {
            let dialog_id = format!("test_dialog_{}", i);
            let config = MediaConfig {
                local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
                remote_addr: None,
                preferred_codec: None,
                parameters: HashMap::new(),
            };

            println!("📞 Creating session: {}", dialog_id);
            controller
                .start_media(DialogId::new(dialog_id.clone()), config)
                .await
                .expect("Failed to start media session");

            let session_info = controller
                .get_session_info(&DialogId::new(dialog_id))
                .await
                .expect("Session should exist");

            println!("✅ Session created with port: {:?}", session_info.rtp_port);
            assert!(session_info.rtp_port.is_some(), "Port should be allocated");

            session_infos.push(session_info);
        }

        // Verify different ports were allocated
        let mut ports = Vec::new();
        for session_info in &session_infos {
            if let Some(port) = session_info.rtp_port {
                ports.push(port);
            }
        }

        // Remove duplicates and check that we have unique ports
        ports.sort();
        ports.dedup();
        assert_eq!(ports.len(), 3, "All sessions should have unique ports");

        println!("🎯 Allocated ports: {:?}", ports);

        // Verify all ports are in valid range (no privileged ports).
        // The upper bound is enforced by the `u16` port type.
        for &port in &ports {
            assert!(port >= 1024, "Port should be >= 1024 (non-privileged)");
        }

        println!("✅ All ports are in valid range and unique");

        // Clean up sessions
        for i in 0..3 {
            let dialog_id = format!("test_dialog_{}", i);
            controller
                .stop_media(&DialogId::new(dialog_id))
                .await
                .expect("Failed to stop media session");
        }

        println!("✨ Dynamic port allocation test completed successfully!");
        println!("🔧 rtp-core's PortAllocator is providing conflict-free dynamic allocation");
    }

    fn bind_adjacent_port_probe() -> (StdUdpSocket, u16) {
        for _ in 0..100 {
            let held = StdUdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let first = held.local_addr().unwrap().port();
            if first == u16::MAX {
                continue;
            }

            if let Ok(second) = StdUdpSocket::bind((Ipv4Addr::LOCALHOST, first + 1)) {
                drop(second);
                return (held, first);
            }
        }

        panic!("failed to find adjacent UDP ports for retry test");
    }

    fn bind_contiguous_port_block(count: usize) -> (Vec<StdUdpSocket>, u16) {
        assert!(count > 1);
        for _ in 0..1_000 {
            let first = StdUdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let base_port = first.local_addr().unwrap().port();
            let Ok(last_offset) = u16::try_from(count - 1) else {
                break;
            };
            if base_port.checked_add(last_offset).is_none() {
                continue;
            }

            let mut sockets = vec![first];
            let mut complete = true;
            for offset in 1..count {
                let port = base_port + u16::try_from(offset).unwrap();
                match StdUdpSocket::bind((Ipv4Addr::LOCALHOST, port)) {
                    Ok(socket) => sockets.push(socket),
                    Err(_) => {
                        complete = false;
                        break;
                    }
                }
            }
            if complete {
                return (sockets, base_port);
            }
        }

        panic!("failed to find {count} contiguous UDP ports for retry test");
    }

    #[tokio::test]
    async fn controllers_with_same_bind_domain_share_port_reservations() {
        let (range_probe, base_port) = bind_adjacent_port_probe();
        drop(range_probe);

        let first_controller = MediaSessionController::with_port_range(base_port, base_port + 1);
        let second_controller = MediaSessionController::with_port_range(base_port, base_port + 1);
        let dialog_id = DialogId::new("same-dialog-id");
        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            remote_addr: None,
            preferred_codec: None,
            parameters: HashMap::new(),
        };

        first_controller
            .start_media(dialog_id.clone(), config.clone())
            .await
            .expect("first controller media start");
        second_controller
            .start_media(dialog_id.clone(), config)
            .await
            .expect("second controller media start");

        let first_port = first_controller
            .get_session_info(&dialog_id)
            .await
            .expect("first session info")
            .rtp_port;
        let second_port = second_controller
            .get_session_info(&dialog_id)
            .await
            .expect("second session info")
            .rtp_port;
        assert_ne!(first_port, second_port);

        first_controller
            .stop_media(&dialog_id)
            .await
            .expect("stop first controller media");
        assert!(second_controller
            .get_session_info(&dialog_id)
            .await
            .is_some());
        second_controller
            .stop_media(&dialog_id)
            .await
            .expect("stop second controller media");
    }

    #[tokio::test]
    async fn test_start_media_retries_when_reserved_port_bind_fails() {
        let (_held_socket, occupied_port) = bind_adjacent_port_probe();
        let mut controller = MediaSessionController::new();
        let mut port_config = PortAllocatorConfig::default();
        port_config.port_range_start = occupied_port;
        port_config.port_range_end = occupied_port + 1;
        port_config.allocation_strategy = AllocationStrategy::Incremental;
        port_config.validate_ports = false;
        controller.port_allocator = Some(Arc::new(PortAllocator::with_config(port_config)));

        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            remote_addr: None,
            preferred_codec: None,
            parameters: HashMap::new(),
        };

        let dialog_id = DialogId::new("retry_bind_conflict");
        controller
            .start_media(dialog_id.clone(), config)
            .await
            .expect("start_media should retry the next reserved port");

        let session_info = controller
            .get_session_info(&dialog_id)
            .await
            .expect("session should exist after retry");
        assert_eq!(session_info.rtp_port, Some(occupied_port + 1));
        assert_eq!(controller.allocated_port_count().await, 1);

        controller
            .stop_media(&dialog_id)
            .await
            .expect("session should stop cleanly");
        assert_eq!(controller.allocated_port_count().await, 0);
    }

    #[tokio::test]
    async fn start_media_scans_beyond_eight_bind_collisions() {
        const RANGE_LEN: usize = 12;
        let (mut held_sockets, base_port) = bind_contiguous_port_block(RANGE_LEN);
        let expected_port = base_port + u16::try_from(RANGE_LEN - 1).unwrap();
        drop(held_sockets.pop());

        let mut controller = MediaSessionController::new();
        let mut port_config = PortAllocatorConfig::default();
        port_config.port_range_start = base_port;
        port_config.port_range_end = expected_port;
        port_config.allocation_strategy = AllocationStrategy::Incremental;
        port_config.pairing_strategy = PairingStrategy::Muxed;
        port_config.prefer_port_reuse = false;
        port_config.validate_ports = false;
        controller.port_allocator = Some(Arc::new(PortAllocator::with_config(port_config)));

        let dialog_id = DialogId::new("full_range_bind_retry");
        controller
            .start_media(
                dialog_id.clone(),
                MediaConfig {
                    local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
                    remote_addr: None,
                    preferred_codec: None,
                    parameters: HashMap::new(),
                },
            )
            .await
            .expect("one complete range scan should reach the free candidate");

        let session_info = controller
            .get_session_info(&dialog_id)
            .await
            .expect("session after range scan");
        assert_eq!(session_info.rtp_port, Some(expected_port));

        controller
            .stop_media(&dialog_id)
            .await
            .expect("session should stop cleanly");
        drop(held_sockets);
    }

    #[tokio::test]
    async fn test_pass_through_media_flow_does_not_spawn_transmitter() {
        let controller = MediaSessionController::new();
        let dialog_id = DialogId::new("pass_through_no_tx_task");
        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            remote_addr: None,
            preferred_codec: None,
            parameters: HashMap::new(),
        };

        controller
            .start_media(dialog_id.clone(), config)
            .await
            .expect("media session should start");
        controller
            .establish_media_flow(
                &dialog_id,
                SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 40000),
            )
            .await
            .expect("pass-through media flow should establish");

        let wrapper = controller
            .rtp_sessions
            .get(&dialog_id)
            .expect("rtp session should exist");
        assert!(
            wrapper.transmission_enabled,
            "pass-through keeps external RTP frame transmission enabled"
        );
        assert!(
            wrapper.audio_transmitter.is_none(),
            "default pass-through must not spawn a periodic audio transmitter"
        );
        drop(wrapper);

        controller
            .stop_media(&dialog_id)
            .await
            .expect("session should stop cleanly");
    }

    #[tokio::test]
    async fn test_codec_negotiation_pcmu() {
        println!("🧪 Testing PCMU codec negotiation");

        let controller = MediaSessionController::new();

        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("PCMU".to_string()),
            parameters: HashMap::new(),
        };

        // Start session with PCMU codec
        let result = controller
            .start_media(DialogId::new("pcmu_dialog"), config)
            .await;
        assert!(
            result.is_ok(),
            "Should successfully start session with PCMU codec"
        );

        // Verify session was created with PCMU codec
        let session_info = controller
            .get_session_info(&DialogId::new("pcmu_dialog"))
            .await;
        assert!(session_info.is_some());
        let session_info = session_info.unwrap();

        // Check that the preferred codec is stored correctly
        assert_eq!(
            session_info.config.preferred_codec,
            Some("PCMU".to_string())
        );

        println!("✅ PCMU codec negotiation test completed");

        // Cleanup
        controller
            .stop_media(&DialogId::new("pcmu_dialog"))
            .await
            .unwrap();
    }

    #[cfg(feature = "opus")]
    #[tokio::test]
    async fn test_codec_negotiation_opus() {
        println!("🧪 Testing Opus codec negotiation");

        let controller = MediaSessionController::new();

        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("opus".to_string()),
            parameters: HashMap::new(),
        };

        // Start session with Opus codec
        let result = controller
            .start_media(DialogId::new("opus_dialog"), config)
            .await;
        assert!(
            result.is_ok(),
            "Should successfully start session with Opus codec"
        );

        // Verify session was created with Opus codec
        let session_info = controller
            .get_session_info(&DialogId::new("opus_dialog"))
            .await;
        assert!(session_info.is_some());
        let session_info = session_info.unwrap();

        // Check that the preferred codec is stored correctly
        assert_eq!(
            session_info.config.preferred_codec,
            Some("opus".to_string())
        );

        println!("✅ Opus codec negotiation test completed");

        // Cleanup
        controller
            .stop_media(&DialogId::new("opus_dialog"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_unknown_codec_fails_without_state_mutation() {
        let controller = MediaSessionController::new();

        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("unknown_codec".to_string()),
            parameters: HashMap::new(),
        };

        let result = controller
            .start_media(DialogId::new("fallback_dialog"), config)
            .await;
        assert!(matches!(
            result,
            Err(Error::Codec(
                crate::error::CodecError::UnsupportedCodec { .. }
            ))
        ));
        assert!(controller
            .get_session_info(&DialogId::new("fallback_dialog"))
            .await
            .is_none());
    }

    #[tokio::test]
    async fn test_codec_negotiation_default() {
        println!("🧪 Testing default codec negotiation (no preferred codec)");

        let controller = MediaSessionController::new();

        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: None, // No preferred codec
            parameters: HashMap::new(),
        };

        // Start session with no preferred codec (should default to PCMU)
        let result = controller
            .start_media(DialogId::new("default_dialog"), config)
            .await;
        assert!(
            result.is_ok(),
            "Should successfully start session with default codec"
        );

        // Verify session was created
        let session_info = controller
            .get_session_info(&DialogId::new("default_dialog"))
            .await;
        assert!(session_info.is_some());
        let session_info = session_info.unwrap();

        // Check that no preferred codec is set
        assert_eq!(session_info.config.preferred_codec, None);

        println!("✅ Default codec negotiation test completed");

        // Cleanup
        controller
            .stop_media(&DialogId::new("default_dialog"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_codec_case_insensitive() {
        println!("🧪 Testing case-insensitive codec negotiation");

        let controller = MediaSessionController::new();

        // Test different case variations
        let test_cases = vec![("pcmu", "pcmu"), ("PCMU", "PCMU"), ("PcMu", "PcMu")];
        #[cfg(feature = "opus")]
        let test_cases = {
            let mut test_cases = test_cases;
            test_cases.extend([("opus", "opus"), ("Opus", "Opus"), ("OPUS", "OPUS")]);
            test_cases
        };

        for (i, (codec_name, expected_stored)) in test_cases.into_iter().enumerate() {
            let dialog_id = format!("case_test_{}", i);

            let config = MediaConfig {
                local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
                remote_addr: None,
                preferred_codec: Some(codec_name.to_string()),
                parameters: HashMap::new(),
            };

            // Start session with case variation
            let result = controller
                .start_media(DialogId::new(dialog_id.clone()), config)
                .await;
            assert!(
                result.is_ok(),
                "Should successfully start session with codec: {}",
                codec_name
            );

            // Verify session was created
            let session_info = controller
                .get_session_info(&DialogId::new(dialog_id.clone()))
                .await;
            assert!(session_info.is_some());
            let session_info = session_info.unwrap();

            // Check that the original case is preserved
            assert_eq!(
                session_info.config.preferred_codec,
                Some(expected_stored.to_string())
            );

            // Cleanup
            controller
                .stop_media(&DialogId::new(dialog_id))
                .await
                .unwrap();
        }

        println!("✅ Case-insensitive codec negotiation test completed");
    }

    #[tokio::test]
    async fn test_codec_negotiation_pcma() {
        println!("🧪 Testing PCMA (G.711 A-law) codec negotiation");

        let controller = MediaSessionController::new();

        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("PCMA".to_string()),
            parameters: HashMap::new(),
        };

        // Start session with PCMA codec
        let result = controller
            .start_media(DialogId::new("pcma_dialog"), config)
            .await;
        assert!(
            result.is_ok(),
            "Should successfully start session with PCMA codec"
        );

        // Verify session was created with PCMA codec
        let session_info = controller
            .get_session_info(&DialogId::new("pcma_dialog"))
            .await;
        assert!(session_info.is_some());
        let session_info = session_info.unwrap();

        // Check that the preferred codec is stored correctly
        assert_eq!(
            session_info.config.preferred_codec,
            Some("PCMA".to_string())
        );

        println!("✅ PCMA (G.711 A-law) codec negotiation test completed");

        // Cleanup
        controller
            .stop_media(&DialogId::new("pcma_dialog"))
            .await
            .unwrap();
    }

    #[cfg(feature = "g729")]
    #[tokio::test]
    async fn test_codec_negotiation_g729() {
        println!("🧪 Testing G729 codec negotiation");

        let controller = MediaSessionController::new();

        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("G729".to_string()),
            parameters: HashMap::new(),
        };

        // Start session with G729 codec
        let result = controller
            .start_media(DialogId::new("g729_dialog"), config)
            .await;
        assert!(
            result.is_ok(),
            "Should successfully start session with G729 codec"
        );

        // Verify session was created with G729 codec
        let session_info = controller
            .get_session_info(&DialogId::new("g729_dialog"))
            .await;
        assert!(session_info.is_some());
        let session_info = session_info.unwrap();

        // Check that the preferred codec is stored correctly
        assert_eq!(
            session_info.config.preferred_codec,
            Some("G729".to_string())
        );

        println!("✅ G729 codec negotiation test completed");

        // Cleanup
        controller
            .stop_media(&DialogId::new("g729_dialog"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_all_g711_variants() {
        println!("🧪 Testing all G.711 variants comprehensively");

        let controller = MediaSessionController::new();

        // Test G.711 μ-law (PCMU)
        let pcmu_config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("PCMU".to_string()),
            parameters: HashMap::new(),
        };

        controller
            .start_media(DialogId::new("g711_mulaw"), pcmu_config)
            .await
            .unwrap();
        let pcmu_info = controller
            .get_session_info(&DialogId::new("g711_mulaw"))
            .await
            .unwrap();
        assert_eq!(pcmu_info.config.preferred_codec, Some("PCMU".to_string()));

        // Test G.711 A-law (PCMA)
        let pcma_config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("PCMA".to_string()),
            parameters: HashMap::new(),
        };

        controller
            .start_media(DialogId::new("g711_alaw"), pcma_config)
            .await
            .unwrap();
        let pcma_info = controller
            .get_session_info(&DialogId::new("g711_alaw"))
            .await
            .unwrap();
        assert_eq!(pcma_info.config.preferred_codec, Some("PCMA".to_string()));

        println!("✅ Verified both G.711 variants:");
        println!("   - PCMU (μ-law): payload type 0, 8000Hz");
        println!("   - PCMA (A-law): payload type 8, 8000Hz");

        // Cleanup
        controller
            .stop_media(&DialogId::new("g711_mulaw"))
            .await
            .unwrap();
        controller
            .stop_media(&DialogId::new("g711_alaw"))
            .await
            .unwrap();

        println!("✅ All G.711 variants test completed");
    }

    #[tokio::test]
    async fn test_comprehensive_codec_matrix() {
        println!("🧪 Testing comprehensive codec support matrix");

        let controller = MediaSessionController::new();

        // Test all supported codecs with their expected payload types and clock rates
        let test_cases = [
            ("PCMU", 0, 8000, "G.711 μ-law"),
            ("PCMA", 8, 8000, "G.711 A-law"),
            #[cfg(feature = "g729")]
            ("G729", 18, 8000, "G.729"),
            #[cfg(feature = "opus")]
            ("opus", 111, 48000, "Opus"),
        ];

        for (codec_name, expected_pt, expected_clock, description) in test_cases {
            let dialog_id = format!("codec_matrix_{}", codec_name.to_lowercase());

            let config = MediaConfig {
                local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
                remote_addr: None,
                preferred_codec: Some(codec_name.to_string()),
                parameters: HashMap::new(),
            };

            println!(
                "  Testing {}: {} (PT:{}, {}Hz)",
                codec_name, description, expected_pt, expected_clock
            );

            // Start session
            let result = controller
                .start_media(DialogId::new(dialog_id.clone()), config)
                .await;
            assert!(
                result.is_ok(),
                "Should successfully start session with {}",
                codec_name
            );

            // Verify codec mapping (indirectly through successful session creation)
            let session_info = controller
                .get_session_info(&DialogId::new(dialog_id.clone()))
                .await;
            assert!(session_info.is_some());
            let session_info = session_info.unwrap();
            assert_eq!(
                session_info.config.preferred_codec,
                Some(codec_name.to_string())
            );

            // Cleanup
            controller
                .stop_media(&DialogId::new(dialog_id))
                .await
                .unwrap();
        }

        println!("✅ Comprehensive codec matrix test completed");
        println!("   All RFC 3551 static codecs and Opus tested successfully!");
    }

    #[cfg(feature = "opus")]
    #[tokio::test]
    async fn test_update_media_codec_change() {
        println!("🧪 Testing codec change in update_media");

        let controller = MediaSessionController::new();

        // Start session with PCMU
        let initial_config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("PCMU".to_string()),
            parameters: HashMap::new(),
        };

        let dialog_id = DialogId::new("codec_change_dialog");
        let result = controller
            .start_media(dialog_id.clone(), initial_config)
            .await;
        assert!(
            result.is_ok(),
            "Should successfully start session with PCMU"
        );

        // Verify initial codec
        let session_info = controller.get_session_info(&dialog_id).await;
        assert!(session_info.is_some());
        assert_eq!(
            session_info.unwrap().config.preferred_codec,
            Some("PCMU".to_string())
        );

        // Update to Opus codec
        let updated_config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("opus".to_string()),
            parameters: HashMap::new(),
        };

        let result = controller
            .update_media(dialog_id.clone(), updated_config)
            .await;
        assert!(result.is_ok(), "Should successfully update codec to Opus");

        // Verify codec was updated
        let session_info = controller.get_session_info(&dialog_id).await;
        assert!(session_info.is_some());
        assert_eq!(
            session_info.unwrap().config.preferred_codec,
            Some("opus".to_string())
        );

        println!("✅ Codec change test completed successfully!");
    }

    #[tokio::test]
    async fn test_update_media_combined_changes() {
        println!("🧪 Testing combined remote address and codec change");

        let controller = MediaSessionController::new();

        // Start session with no remote address and PCMU
        let initial_config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("PCMU".to_string()),
            parameters: HashMap::new(),
        };

        let dialog_id = DialogId::new("combined_change_dialog");
        let result = controller
            .start_media(dialog_id.clone(), initial_config)
            .await;
        assert!(result.is_ok(), "Should successfully start session");

        // Update both remote address and codec
        let remote_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 5060);
        let updated_config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: Some(remote_addr),
            preferred_codec: Some("PCMA".to_string()),
            parameters: HashMap::new(),
        };

        let result = controller
            .update_media(dialog_id.clone(), updated_config)
            .await;
        assert!(
            result.is_ok(),
            "Should successfully update both address and codec"
        );

        // Verify both changes were applied
        let session_info = controller.get_session_info(&dialog_id).await;
        assert!(session_info.is_some());
        let info = session_info.unwrap();
        assert_eq!(info.config.remote_addr, Some(remote_addr));
        assert_eq!(info.config.preferred_codec, Some("PCMA".to_string()));

        println!("✅ Combined change test completed successfully!");
    }

    #[tokio::test]
    async fn update_media_waits_for_the_dialog_generation_lock() {
        let controller = Arc::new(MediaSessionController::new());
        let dialog_id = DialogId::new("serialized_codec_update");
        controller
            .start_media(
                dialog_id.clone(),
                MediaConfig {
                    local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
                    remote_addr: None,
                    preferred_codec: Some("PCMU".to_string()),
                    parameters: HashMap::new(),
                },
            )
            .await
            .unwrap();

        let update_lock = controller
            .rtp_sessions
            .get(&dialog_id)
            .map(|wrapper| Arc::clone(&wrapper.update_lock))
            .unwrap();
        let guard = update_lock.lock().await;
        let updating = {
            let controller = Arc::clone(&controller);
            let dialog_id = dialog_id.clone();
            tokio::spawn(async move {
                controller
                    .update_media(
                        dialog_id,
                        MediaConfig {
                            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
                            remote_addr: None,
                            preferred_codec: Some("PCMA".to_string()),
                            parameters: HashMap::new(),
                        },
                    )
                    .await
            })
        };
        tokio::task::yield_now().await;
        assert!(
            !updating.is_finished(),
            "a second codec generation must not pass the dialog update lock"
        );
        drop(guard);
        updating.await.unwrap().unwrap();

        let config = controller.sessions.get(&dialog_id).unwrap().config.clone();
        let runtime_format = controller
            .codec_runtimes
            .get(&dialog_id)
            .unwrap()
            .format
            .clone();
        let session = controller
            .rtp_sessions
            .get(&dialog_id)
            .unwrap()
            .session
            .clone();
        assert_eq!(config.preferred_codec.as_deref(), Some("PCMA"));
        assert_eq!(runtime_format.name, "PCMA");
        assert_eq!(session.lock().await.get_payload_type(), 8);
        controller.stop_media(&dialog_id).await.unwrap();
    }

    #[tokio::test]
    async fn codec_update_rebuilds_generated_audio_transmitter() {
        let peer = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let remote_addr = peer.local_addr().unwrap();
        let controller = MediaSessionController::new();
        let dialog_id = DialogId::new("generated_audio_codec_update");
        controller
            .start_media(
                dialog_id.clone(),
                MediaConfig {
                    local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
                    remote_addr: Some(remote_addr),
                    preferred_codec: Some("PCMU".to_string()),
                    parameters: HashMap::new(),
                },
            )
            .await
            .unwrap();
        controller
            .start_audio_transmission_with_tone(&dialog_id)
            .await
            .unwrap();
        controller
            .set_audio_source(
                &dialog_id,
                AudioSource::CustomSamples {
                    samples: vec![0x7f, 0xff],
                    repeat: true,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            controller
                .rtp_sessions
                .get(&dialog_id)
                .unwrap()
                .audio_transmitter
                .as_ref()
                .unwrap()
                .codec_format()
                .payload_type,
            0
        );

        controller
            .update_media(
                dialog_id.clone(),
                MediaConfig {
                    local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
                    remote_addr: Some(remote_addr),
                    preferred_codec: Some("PCMA".to_string()),
                    parameters: HashMap::new(),
                },
            )
            .await
            .unwrap();
        let wrapper = controller.rtp_sessions.get(&dialog_id).unwrap();
        let transmitter = wrapper.audio_transmitter.as_ref().unwrap();
        assert_eq!(transmitter.codec_format().name, "PCMA");
        assert_eq!(transmitter.codec_format().payload_type, 8);
        match transmitter.replacement_config().source {
            AudioSource::CustomSamples { samples, repeat } => {
                assert_eq!(samples, vec![0x7f, 0xff]);
                assert!(repeat);
            }
            source => panic!("codec update replaced the current generated source: {source:?}"),
        }
        drop(wrapper);

        #[cfg(feature = "opus")]
        {
            controller
                .update_media(
                    dialog_id.clone(),
                    MediaConfig {
                        local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
                        remote_addr: Some(remote_addr),
                        preferred_codec: Some("opus".to_string()),
                        parameters: HashMap::new(),
                    }
                    .with_negotiated_audio_codec("opus", 96, 48_000, 2),
                )
                .await
                .unwrap();
            let wrapper = controller.rtp_sessions.get(&dialog_id).unwrap();
            let transmitter = wrapper.audio_transmitter.as_ref().unwrap();
            assert_eq!(transmitter.codec_format().name, "opus");
            assert_eq!(transmitter.codec_format().payload_type, 96);
            assert_eq!(transmitter.codec_format().clock_rate, 48_000);
        }
        controller.stop_media(&dialog_id).await.unwrap();
    }

    #[tokio::test]
    async fn test_update_media_no_changes() {
        println!("🧪 Testing update_media with no actual changes");

        let controller = MediaSessionController::new();

        // Start session
        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: Some(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)),
                5060,
            )),
            preferred_codec: Some("PCMU".to_string()),
            parameters: HashMap::new(),
        };

        let dialog_id = DialogId::new("no_change_dialog");
        let result = controller
            .start_media(dialog_id.clone(), config.clone())
            .await;
        assert!(result.is_ok(), "Should successfully start session");

        // Update with same config (no changes)
        let result = controller.update_media(dialog_id.clone(), config).await;
        assert!(
            result.is_ok(),
            "Should successfully handle no-change update"
        );

        println!("✅ No-change update test completed successfully!");
    }

    #[cfg(not(feature = "opus"))]
    #[tokio::test]
    async fn disabled_opus_start_and_update_fail_atomically() {
        let controller = MediaSessionController::new();
        let dialog_id = DialogId::new("disabled-opus");
        let base = MediaConfig {
            local_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            remote_addr: None,
            preferred_codec: Some("PCMU".to_string()),
            parameters: HashMap::new(),
        };
        controller
            .start_media(dialog_id.clone(), base.clone())
            .await
            .unwrap();

        let mut unsupported = base;
        unsupported.remote_addr = Some(SocketAddr::from(([203, 0, 113, 10], 9_999)));
        unsupported.preferred_codec = Some("OpUs".to_string());
        assert!(matches!(
            controller
                .update_media(dialog_id.clone(), unsupported)
                .await,
            Err(Error::Codec(
                crate::error::CodecError::UnsupportedCodec { .. }
            ))
        ));
        let stable = controller.get_session_info(&dialog_id).await.unwrap();
        assert_eq!(stable.config.preferred_codec.as_deref(), Some("PCMU"));
        assert_eq!(stable.config.remote_addr, None);
    }

    #[tokio::test]
    async fn g722_is_explicitly_unsupported() {
        let controller = MediaSessionController::new();
        let dialog_id = DialogId::new("g722-unsupported");
        let config = MediaConfig {
            local_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            remote_addr: None,
            preferred_codec: Some("G.722".to_string()),
            parameters: HashMap::new(),
        };
        assert!(matches!(
            controller.start_media(dialog_id.clone(), config).await,
            Err(Error::Codec(
                crate::error::CodecError::UnsupportedCodec { .. }
            ))
        ));
        assert!(controller.get_session_info(&dialog_id).await.is_none());
    }

    #[cfg(feature = "opus")]
    #[tokio::test]
    async fn controller_opus_rtp_round_trip_uses_negotiated_payload_and_clock() {
        let controller = MediaSessionController::with_port_range(31_000, 31_100);
        let sender = DialogId::new("opus-wire-sender");
        let receiver = DialogId::new("opus-wire-receiver");
        let base = |codec: &str| {
            MediaConfig {
                local_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
                remote_addr: None,
                preferred_codec: None,
                parameters: HashMap::new(),
            }
            .with_negotiated_audio_codec(codec, 96, 48_000, 1)
        };
        controller
            .start_media(sender.clone(), base("Opus"))
            .await
            .unwrap();
        controller
            .start_media(receiver.clone(), base("OPUS"))
            .await
            .unwrap();
        let sender_port = controller
            .get_session_info(&sender)
            .await
            .unwrap()
            .rtp_port
            .unwrap();
        let receiver_port = controller
            .get_session_info(&receiver)
            .await
            .unwrap()
            .rtp_port
            .unwrap();

        let mut sender_config = base("opus");
        sender_config.remote_addr = Some(SocketAddr::from(([127, 0, 0, 1], receiver_port)));
        controller
            .update_media(sender.clone(), sender_config)
            .await
            .unwrap();
        let mut receiver_config = base("opus");
        receiver_config.remote_addr = Some(SocketAddr::from(([127, 0, 0, 1], sender_port)));
        controller
            .update_media(receiver.clone(), receiver_config)
            .await
            .unwrap();

        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel(4);
        controller
            .set_audio_frame_callback(receiver.clone(), frame_tx)
            .await
            .unwrap();

        for timestamp in [0, 960] {
            let samples = (0..960)
                .map(|index| (((index as f32 / 20.0).sin()) * 8_000.0) as i16)
                .collect();
            controller
                .encode_and_send_audio(&sender, AudioFrame::new(samples, 48_000, 1, timestamp))
                .await
                .unwrap();
        }

        for expected_timestamp in [0, 960] {
            let frame = tokio::time::timeout(std::time::Duration::from_secs(2), frame_rx.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(frame.sample_rate, 48_000);
            assert_eq!(frame.channels, 1);
            assert_eq!(frame.samples.len(), 960);
            assert_eq!(frame.timestamp, expected_timestamp);
        }

        controller.stop_media(&sender).await.unwrap();
        controller.stop_media(&receiver).await.unwrap();
    }
}
