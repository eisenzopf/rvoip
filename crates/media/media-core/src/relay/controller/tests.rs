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

        controller
            .stop_media(&dialog_id)
            .await
            .expect("session should stop cleanly");
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
