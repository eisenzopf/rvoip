//! Explicit TLS Contact compatibility at the decrypted transport boundary.
use rvoip_sip::api::events::Event;
use rvoip_sip::api::unified::{Config, SipTlsMode, UnifiedCoordinator};
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

const DEADLINE: Duration = Duration::from_secs(10);

enum Wire {
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
    Udp(UdpSocket, std::net::SocketAddr),
}
impl Wire {
    async fn send(&mut self, bytes: &[u8]) {
        match self {
            Self::Tls(stream) => stream.write_all(bytes).await.unwrap(),
            Self::Udp(socket, target) => {
                socket.send_to(bytes, *target).await.unwrap();
            }
        }
    }
    async fn receive(&mut self) -> String {
        let mut bytes = vec![0; 8192];
        let count = tokio::time::timeout(DEADLINE, async {
            match self {
                Self::Tls(stream) => stream.read(&mut bytes).await.unwrap(),
                Self::Udp(socket, _) => socket.recv(&mut bytes).await.unwrap(),
            }
        })
        .await
        .expect("SIP response deadline");
        assert!(count > 0, "signaling connection closed");
        String::from_utf8(bytes[..count].to_vec()).unwrap()
    }
    async fn until(&mut self, expected: &str) -> String {
        tokio::time::timeout(DEADLINE, async {
            let mut text = String::new();
            loop {
                text.push_str(&self.receive().await);
                if text.contains(expected) && text.ends_with("\r\n") {
                    return text;
                }
            }
        })
        .await
        .expect("expected SIP response")
    }
}

fn port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn header<'a>(message: &'a str, name: &str) -> &'a str {
    message
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name).then_some(value.trim())
        })
        .expect("required response header")
}

#[allow(
    clippy::too_many_lines,
    reason = "one wire fixture owns setup, media exchange and teardown for each policy case"
)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tls_contact_compatibility_preserves_transport_and_bidirectional_rtp() {
    // Permit only the explicit TLS Contact, over actual TLS, with opt-in.
    for (tls, compatibility, contact_transport, accepted, local_hangup) in [
        (true, false, ";transport=tls", false, false),
        (true, true, "", false, false),
        (true, true, ";transport=udp", false, false),
        (false, true, ";transport=tls", false, false),
        (true, true, ";transport=tls", true, false),
        (true, true, ";transport=tls", true, true),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let identity = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let cert = directory.path().join("cert.pem");
        let key = directory.path().join("key.pem");
        std::fs::write(&cert, identity.cert.pem()).unwrap();
        std::fs::write(&key, identity.signing_key.serialize_pem()).unwrap();
        let base = port();
        let tls_port = port();
        let mut config = Config::local("appliance", base);
        config.sip_tls_mode = SipTlsMode::ClientAndServer;
        config.tls_bind_addr = Some(([127, 0, 0, 1], tls_port).into());
        config.tls_cert_path = Some(cert);
        config.tls_key_path = Some(key);
        config.sip_advertised_addr = Some(([127, 0, 0, 1], tls_port).into());
        config.tls_advertised_addr = config.sip_advertised_addr;
        config.sip_allow_tls_contact_on_sips = compatibility;
        config.offer_srtp = true;
        config.srtp_required = false;
        let receiver = Box::pin(UnifiedCoordinator::new(config)).await.unwrap();
        let mut events = receiver.events().await.unwrap();
        let mut wire = if tls {
            let mut roots = rustls::RootCertStore::empty();
            roots.add(identity.cert.der().clone()).unwrap();
            let client = rustls::ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
            let stream = TcpStream::connect(("127.0.0.1", tls_port)).await.unwrap();
            let stream = tokio_rustls::TlsConnector::from(Arc::new(client))
                .connect(
                    rustls::pki_types::ServerName::try_from("localhost").unwrap(),
                    stream,
                )
                .await
                .unwrap();
            Wire::Tls(Box::new(stream))
        } else {
            Wire::Udp(
                UdpSocket::bind("127.0.0.1:0").await.unwrap(),
                ([127, 0, 0, 1], base).into(),
            )
        };
        let rtp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let rtp_port = rtp.local_addr().unwrap().port();
        let sdp = format!(
            "v=0\r\no=Jambonz-Mediaserver 1 1 IN IP4 127.0.0.1\r\ns=Jambonz-Mediaserver\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio {rtp_port} RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\na=sendrecv\r\na=ptime:20\r\n"
        );
        // Even the UDP negative case claims TLS in Via: this must not enable compatibility.
        let invite = format!(
            "INVITE sips:appliance@127.0.0.1:{tls_port} SIP/2.0\r\nVia: SIP/2.0/TLS 127.0.0.1:5061;branch=z9hG4bK-handoff-{base};rport\r\nMax-Forwards: 70\r\nFrom: <sip:undefined@127.0.0.1>;tag=handoff\r\nTo: <sips:appliance@127.0.0.1:{tls_port}>\r\nCall-ID: handoff-{base}\r\nCSeq: 1 INVITE\r\nContact: <sip:undefined@127.0.0.1:5061{contact_transport}>\r\nX-Vapi-Correlation-Id: 550e8400-e29b-41d4-a716-446655440000\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{sdp}",
            sdp.len()
        );
        wire.send(invite.as_bytes()).await;
        if !accepted {
            let response = wire.until("SIP/2.0 400 ").await;
            assert!(!response.contains("SIP/2.0 200 "));
            receiver.shutdown();
            continue;
        }
        let call_id = tokio::time::timeout(DEADLINE, async {
            loop {
                if let Event::IncomingCall { call_id, .. } = events.next().await.unwrap() {
                    break call_id;
                }
            }
        })
        .await
        .expect("compatible INVITE reaches application admission");
        receiver.accept_call(&call_id).await.unwrap();
        let mut response = wire.until("SIP/2.0 200 ").await;
        while !response.contains("a=rtpmap:") {
            response.push_str(&wire.receive().await);
        }
        let response = &response[response.find("SIP/2.0 200 ").unwrap()..];
        let contact = header(response, "Contact").trim_matches(['<', '>']);
        assert!(contact.starts_with("sips:"), "our Contact remains secure");
        assert!(response.contains("RTP/AVP"));
        assert!(!response.contains("a=crypto:"));
        let to = header(response, "To");
        let media_port: u16 = response
            .lines()
            .find(|line| line.starts_with("m=audio "))
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let ack = format!(
            "ACK {contact} SIP/2.0\r\nVia: SIP/2.0/TLS 127.0.0.1:5061;branch=z9hG4bK-ack-{base};rport\r\nMax-Forwards: 70\r\nFrom: <sip:undefined@127.0.0.1>;tag=handoff\r\nTo: {to}\r\nCall-ID: handoff-{base}\r\nCSeq: 1 ACK\r\nContent-Length: 0\r\n\r\n"
        );
        wire.send(ack.as_bytes()).await;
        tokio::time::timeout(DEADLINE, async {
            loop {
                if matches!(events.next().await.unwrap(), Event::CallEstablished { .. }) {
                    break;
                }
            }
        })
        .await
        .unwrap();
        let mut decoded = receiver.subscribe_to_audio(&call_id).await.unwrap();
        for sequence in 0..40_u16 {
            let mut packet = vec![0x80, 0];
            packet.extend_from_slice(&sequence.to_be_bytes());
            packet.extend_from_slice(&(u32::from(sequence) * 160).to_be_bytes());
            packet.extend_from_slice(&1234_u32.to_be_bytes());
            packet.extend_from_slice(&[0x90; 160]);
            rtp.send_to(&packet, ("127.0.0.1", media_port))
                .await
                .unwrap();
            receiver
                .send_audio(
                    &call_id,
                    rvoip_sip::AudioFrame::new(vec![3000; 160], 8000, 1, u32::from(sequence) * 160),
                )
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let frame = tokio::time::timeout(DEADLINE, decoded.receiver.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            frame.samples.iter().any(|sample| *sample != 0),
            "incoming RTP decoded to non-silent audio"
        );
        let mut packet = [0; 2048];
        let count = tokio::time::timeout(DEADLINE, rtp.recv(&mut packet))
            .await
            .unwrap()
            .unwrap();
        assert!(count >= 172, "outbound RTP contains audio payload");
        assert_eq!(packet[0] >> 6, 2);
        assert_eq!(packet[1] & 0x7f, 0);
        if local_hangup {
            // The peer advertises 5061 but only has this established TLS flow.
            // A new outbound connection to Contact cannot pass this test.
            let (ended, ()) = tokio::time::timeout(DEADLINE, async {
                tokio::join!(receiver.hangup(&call_id), async {
                    let request = wire.until("BYE ").await;
                    assert!(request.starts_with("BYE sips:undefined@127.0.0.1:5061;transport=tls SIP/2.0"));
                    assert!(header(&request, "Via").starts_with("SIP/2.0/TLS "));
                    let reply = format!(
                        "SIP/2.0 200 OK\r\nVia: {}\r\nFrom: {}\r\nTo: {}\r\nCall-ID: {}\r\nCSeq: {}\r\nContent-Length: 0\r\n\r\n",
                        header(&request, "Via"), header(&request, "From"), header(&request, "To"),
                        header(&request, "Call-ID"), header(&request, "CSeq"),
                    );
                    wire.send(reply.as_bytes()).await;
                })
            }).await.expect("local BYE and final response complete promptly");
            ended.expect("local hangup is confirmed on the admitted TLS flow");
            receiver.shutdown();
            continue;
        }
        let bye = format!(
            "BYE {contact} SIP/2.0\r\nVia: SIP/2.0/TLS 127.0.0.1:5061;branch=z9hG4bK-bye-{base};rport\r\nMax-Forwards: 70\r\nFrom: <sip:undefined@127.0.0.1>;tag=handoff\r\nTo: {to}\r\nCall-ID: handoff-{base}\r\nCSeq: 2 BYE\r\nContent-Length: 0\r\n\r\n"
        );
        wire.send(bye.as_bytes()).await;
        wire.until("SIP/2.0 200 ").await;
        receiver.shutdown();
    }
}

#[test]
fn compatible_contact_keeps_secure_remote_target_and_rejects_downgrades() {
    use rvoip_sip_dialog::dialog::Dialog;
    let mut dialog = Dialog::new_early(
        "compatibility-target".into(),
        "sips:appliance@example.test".parse().unwrap(),
        "sip:undefined@peer.example.test".parse().unwrap(),
        None,
        Some("peer".into()),
        false,
    );
    let remote = "sip:undefined@peer.example.test:5061;transport=tls";
    assert!(!dialog.update_remote_target(remote.parse().unwrap()));
    dialog.allow_tls_contact_on_sips = true;
    assert!(dialog.update_remote_target(remote.parse().unwrap()));
    assert_eq!(
        dialog.remote_target.to_string(),
        "sips:undefined@peer.example.test:5061;transport=tls"
    );
    assert!(dialog.secure_transport_required);
    let pinned = dialog.remote_target.clone();
    for bad in [
        "sip:peer@other.example.test",
        "sip:peer@other.example.test;transport=udp",
        "sip:peer@other.example.test;transport=tcp",
    ] {
        assert!(!dialog.update_remote_target(bad.parse().unwrap()));
        assert_eq!(dialog.remote_target, pinned);
    }
}
