//! End-to-end bind-versus-advertise coverage for container/NAT deployments.

use std::time::Duration;

use rvoip_sip::{Config, UnifiedCoordinator};
use tokio::net::UdpSocket;

#[tokio::test]
async fn private_bind_uses_public_signaling_and_media_addresses_on_wire() {
    let capture = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind capture socket");
    let capture_port = capture.local_addr().unwrap().port();

    let reservation = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("reserve SIP port");
    let sip_port = reservation.local_addr().unwrap().port();
    drop(reservation);

    let signaling = format!("203.0.113.10:{sip_port}").parse().unwrap();
    let media = "203.0.113.20:40000".parse().unwrap();
    let mut config = Config::on("bridge", "127.0.0.1".parse().unwrap(), sip_port)
        .with_sip_advertised_addr(signaling)
        .with_media_public_addr(media)
        .with_signaling_only_media(40_000);
    config.bind_addr = format!("0.0.0.0:{sip_port}").parse().unwrap();

    let coordinator = UnifiedCoordinator::new(config)
        .await
        .expect("start coordinator");
    tokio::time::sleep(Duration::from_millis(50)).await;

    let target = format!("sip:capture@127.0.0.1:{capture_port}");
    let _call = coordinator
        .invite(Some(format!("sip:bridge@203.0.113.10:{sip_port}")), target)
        .send()
        .await
        .expect("send INVITE");

    let mut packet = vec![0_u8; 16 * 1024];
    let (size, _) = tokio::time::timeout(Duration::from_secs(2), capture.recv_from(&mut packet))
        .await
        .expect("INVITE timeout")
        .expect("receive INVITE");
    let wire = String::from_utf8_lossy(&packet[..size]);

    assert!(wire.starts_with("INVITE "), "unexpected packet: {wire}");
    assert!(
        wire.contains(&format!("Via: SIP/2.0/UDP 203.0.113.10:{sip_port}")),
        "Via did not use advertised signaling address: {wire}"
    );
    assert!(
        wire.contains(&format!("Contact: <sip:bridge@203.0.113.10:{sip_port}")),
        "Contact did not use advertised signaling address: {wire}"
    );
    assert!(
        wire.contains("c=IN IP4 203.0.113.20"),
        "SDP did not use advertised media IP: {wire}"
    );
    assert!(
        wire.contains("m=audio 40000 RTP/AVP"),
        "SDP did not use advertised media port: {wire}"
    );
    assert!(
        !wire.contains("0.0.0.0"),
        "bind wildcard leaked onto the SIP or SDP wire: {wire}"
    );

    coordinator
        .shutdown_gracefully(Some(Duration::from_secs(1)))
        .await
        .expect("shutdown coordinator");
}
