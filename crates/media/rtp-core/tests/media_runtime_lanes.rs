//! The process-wide media executor split into lanes.
//!
//! The executor is global and configured once, so this binary holds a single
//! test that installs a two-lane layout before anything starts the executor.

use std::time::Duration;

use bytes::Bytes;
use rvoip_rtp_core::task_runtime::{
    install_media_runtime_config, media_runtime_snapshot, MediaLanes, MediaRuntimeConfig,
    MediaRuntimeConfigError,
};
use rvoip_rtp_core::{
    RtpEvent, RtpHeader, RtpPacket, RtpTransport, RtpTransportConfig, UdpRtpTransport,
};

fn loopback_config(name: &str) -> RtpTransportConfig {
    RtpTransportConfig {
        local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
        local_rtcp_addr: None,
        symmetric_rtp: false,
        rtcp_mux: true,
        session_id: Some(name.to_string()),
        use_port_allocator: false,
        buffer_config: Default::default(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn installed_lanes_place_transports_round_robin_and_every_transport_receives() {
    let config = MediaRuntimeConfig::new(MediaLanes::Shared {
        worker_threads: Some(4),
        lanes: 2,
    });
    assert!(media_runtime_snapshot().is_none());
    install_media_runtime_config(config.clone()).unwrap();
    // Several components may install the same layout.
    install_media_runtime_config(config.clone()).unwrap();
    assert!(matches!(
        install_media_runtime_config(MediaRuntimeConfig::default()),
        Err(MediaRuntimeConfigError::AlreadyConfigured { active }) if *active == config
    ));
    assert!(
        media_runtime_snapshot().is_none(),
        "installing a layout does not start the executor"
    );

    let mut transports = Vec::new();
    for index in 0..4 {
        transports.push(
            UdpRtpTransport::new(loopback_config(&format!("lane_transport_{index}")))
                .await
                .unwrap(),
        );
    }

    let snapshot = media_runtime_snapshot().expect("transports started the executor");
    assert_eq!(snapshot.config, config);
    let lanes = snapshot
        .lanes
        .iter()
        .map(|lane| (lane.worker_threads, lane.transports_placed))
        .collect::<Vec<_>>();
    assert_eq!(
        lanes,
        [(2, 2), (2, 2)],
        "4 workers and 4 transports, split evenly"
    );

    let sender = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    for (index, transport) in transports.iter().enumerate() {
        let mut events = transport.subscribe();
        let payload = Bytes::from(format!("lane payload {index}"));
        let packet = RtpPacket::new(RtpHeader::new(0, 1, 160, 0x5150_0000), payload.clone());
        sender
            .send_to(
                &packet.serialize().unwrap(),
                transport.local_rtp_addr().unwrap(),
            )
            .await
            .unwrap();
        match tokio::time::timeout(Duration::from_secs(2), events.recv()).await {
            Ok(Ok(RtpEvent::MediaReceived {
                payload: received, ..
            })) => assert_eq!(received, payload),
            other => panic!("transport {index} did not receive its packet: {other:?}"),
        }
    }

    for transport in &transports {
        transport.close().await.unwrap();
    }
}
