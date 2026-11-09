//! `Config::sip_new_call_admission` and
//! `Config::server_call_admission_early_reject` reach the SIP transaction
//! layer: rejected INVITEs get a stateless 503 before any session exists.
use rvoip_sip::api::unified::{Config, SipTlsMode, UnifiedCoordinator};
use rvoip_sip_dialog::transaction::{
    AdmissionLevel, AdmissionMode, NewCallAdmissionConfig, NewCallRateLimit,
};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

const DEADLINE: Duration = Duration::from_secs(10);
const OVERLOAD_TAG: &str = "tag=rvoip-ovl-";

fn port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn invite(transport: &str, target: SocketAddr, local: SocketAddr, call_id: &str) -> String {
    let scheme = if transport == "TLS" { "sips" } else { "sip" };
    format!(
        "INVITE {scheme}:appliance@{target} SIP/2.0\r\n\
         Via: SIP/2.0/{transport} {local};branch=z9hG4bK-{call_id}\r\n\
         Max-Forwards: 70\r\n\
         From: <{scheme}:caller@{local}>;tag=caller-{call_id}\r\n\
         To: <{scheme}:appliance@{target}>\r\n\
         Contact: <{scheme}:caller@{local}>\r\n\
         Call-ID: {call_id}\r\n\
         CSeq: 1 INVITE\r\n\
         Content-Length: 0\r\n\r\n"
    )
}

fn ack(
    transport: &str,
    target: SocketAddr,
    local: SocketAddr,
    call_id: &str,
    rejection: &str,
) -> String {
    let scheme = if transport == "TLS" { "sips" } else { "sip" };
    let to = header(rejection, "To").expect("503 To header");
    format!(
        "ACK {scheme}:appliance@{target} SIP/2.0\r\n\
         Via: SIP/2.0/{transport} {local};branch=z9hG4bK-{call_id}\r\n\
         Max-Forwards: 70\r\n\
         From: <{scheme}:caller@{local}>;tag=caller-{call_id}\r\n\
         To: {to}\r\n\
         Call-ID: {call_id}\r\n\
         CSeq: 1 ACK\r\n\
         Content-Length: 0\r\n\r\n"
    )
}

fn header<'a>(message: &'a str, name: &str) -> Option<&'a str> {
    message.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then_some(value.trim())
    })
}

async fn recv_final(socket: &UdpSocket) -> String {
    tokio::time::timeout(DEADLINE, async {
        let mut bytes = vec![0; 8192];
        loop {
            let (count, _) = socket.recv_from(&mut bytes).await.unwrap();
            let text = String::from_utf8_lossy(&bytes[..count]).into_owned();
            if !text.starts_with("SIP/2.0 1") {
                return text;
            }
        }
    })
    .await
    .expect("final response deadline")
}

async fn wait_for(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(DEADLINE, async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("condition deadline");
}

#[test]
fn new_call_admission_and_early_reject_default_off_and_are_configurable() {
    let config = Config::local("alice", 5060);
    assert_eq!(config.sip_new_call_admission.mode, AdmissionMode::Off);
    assert_eq!(config.sip_new_call_admission.rate_limit, None);
    assert!(!config.server_call_admission_early_reject);

    let tuned = Config::local("alice", 5060)
        .with_sip_new_call_admission(
            NewCallAdmissionConfig::new(AdmissionMode::Enforce)
                .with_rate_limit(NewCallRateLimit::new(250.0, 50.0)),
        )
        .with_server_call_admission_early_reject(true);
    assert_eq!(tuned.sip_new_call_admission.mode, AdmissionMode::Enforce);
    assert_eq!(
        tuned.sip_new_call_admission.rate_limit,
        Some(NewCallRateLimit::new(250.0, 50.0))
    );
    assert!(tuned.server_call_admission_early_reject);
    let debug = format!("{tuned:?}");
    assert!(debug.contains("sip_new_call_admission"));
    assert!(debug.contains("server_call_admission_early_reject"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn udp_invite_rejected_by_the_gate_gets_a_stateless_503_and_its_ack_is_absorbed() {
    let sip_port = port();
    let config = Config::local("appliance", sip_port)
        .with_sip_new_call_admission(NewCallAdmissionConfig::new(AdmissionMode::Enforce));
    let coordinator = Box::pin(UnifiedCoordinator::new(config)).await.unwrap();
    let gate = coordinator.new_call_admission();
    assert_eq!(gate.mode(), AdmissionMode::Enforce);
    gate.set_level(AdmissionLevel::closed(Some(6)));

    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let local = socket.local_addr().unwrap();
    let target: SocketAddr = ([127, 0, 0, 1], sip_port).into();
    let call_id = format!("admission-udp-{sip_port}");
    socket
        .send_to(invite("UDP", target, local, &call_id).as_bytes(), target)
        .await
        .unwrap();
    let rejection = recv_final(&socket).await;
    assert!(rejection.starts_with("SIP/2.0 503"), "{rejection}");
    assert_eq!(header(&rejection, "Retry-After"), Some("6"), "{rejection}");
    assert!(rejection.contains(OVERLOAD_TAG), "{rejection}");
    assert!(rejection.contains(&call_id), "{rejection}");

    socket
        .send_to(
            ack("UDP", target, local, &call_id, &rejection).as_bytes(),
            target,
        )
        .await
        .unwrap();
    wait_for(|| gate.stats().acks_absorbed == 1).await;
    let stats = gate.stats();
    assert_eq!(stats.rejected, 1);
    assert_eq!(stats.tls_ingress_rejected, 0);
    assert!(!stats.tls_ingress_attached);

    coordinator.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tls_invite_is_rejected_on_the_connection_reader() {
    let directory = tempfile::tempdir().unwrap();
    let identity = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert = directory.path().join("cert.pem");
    let key = directory.path().join("key.pem");
    std::fs::write(&cert, identity.cert.pem()).unwrap();
    std::fs::write(&key, identity.signing_key.serialize_pem()).unwrap();

    let tls_port = port();
    let mut config = Config::local("appliance", port());
    config.sip_tls_mode = SipTlsMode::ClientAndServer;
    config.tls_bind_addr = Some(([127, 0, 0, 1], tls_port).into());
    config.tls_cert_path = Some(cert);
    config.tls_key_path = Some(key);
    let config =
        config.with_sip_new_call_admission(NewCallAdmissionConfig::new(AdmissionMode::Enforce));
    let coordinator = Box::pin(UnifiedCoordinator::new(config)).await.unwrap();
    let gate = coordinator.new_call_admission();
    assert!(gate.stats().tls_ingress_attached);
    gate.set_level(AdmissionLevel::closed(Some(8)));

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
    let local = stream.local_addr().unwrap();
    let mut stream = tokio_rustls::TlsConnector::from(Arc::new(client))
        .connect(
            rustls::pki_types::ServerName::try_from("localhost").unwrap(),
            stream,
        )
        .await
        .unwrap();

    let target: SocketAddr = ([127, 0, 0, 1], tls_port).into();
    let call_id = format!("admission-tls-{tls_port}");
    stream
        .write_all(invite("TLS", target, local, &call_id).as_bytes())
        .await
        .unwrap();
    let rejection = tokio::time::timeout(DEADLINE, async {
        let mut text = String::new();
        let mut bytes = vec![0; 8192];
        loop {
            let count = stream.read(&mut bytes).await.unwrap();
            assert!(count > 0, "signaling connection closed");
            text.push_str(std::str::from_utf8(&bytes[..count]).unwrap());
            if text.contains("\r\n\r\n") {
                return text;
            }
        }
    })
    .await
    .expect("503 deadline");
    assert!(rejection.starts_with("SIP/2.0 503"), "{rejection}");
    assert_eq!(header(&rejection, "Retry-After"), Some("8"), "{rejection}");
    assert!(rejection.contains(OVERLOAD_TAG), "{rejection}");

    stream
        .write_all(ack("TLS", target, local, &call_id, &rejection).as_bytes())
        .await
        .unwrap();
    wait_for(|| gate.stats().acks_absorbed == 1).await;
    let stats = gate.stats();
    assert_eq!(stats.tls_ingress_rejected, 1);
    assert_eq!(stats.rejected, 1);
    assert_eq!(stats.decisions, 1, "a TLS INVITE is decided once");

    coordinator.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn early_reject_answers_invites_at_the_admission_limit_statelessly() {
    let sip_port = port();
    let config = Config::local("appliance", sip_port)
        .with_server_call_admission_limit(1)
        .with_server_overload_retry_after_secs(5)
        .with_server_call_admission_early_reject(true);
    let coordinator = Box::pin(UnifiedCoordinator::new(config)).await.unwrap();
    assert_eq!(
        coordinator.new_call_admission().mode(),
        AdmissionMode::Off,
        "the slot cap does not need the gate"
    );

    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let local = socket.local_addr().unwrap();
    let target: SocketAddr = ([127, 0, 0, 1], sip_port).into();

    // The first call occupies the only slot; the application never answers.
    socket
        .send_to(
            invite(
                "UDP",
                target,
                local,
                &format!("early-reject-held-{sip_port}"),
            )
            .as_bytes(),
            target,
        )
        .await
        .unwrap();

    tokio::time::timeout(DEADLINE, async {
        while coordinator.list_sessions().await.is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the held call creates its session");

    // With the only slot taken, the next new INVITE is refused statelessly.
    let call_id = format!("early-reject-{sip_port}");
    socket
        .send_to(invite("UDP", target, local, &call_id).as_bytes(), target)
        .await
        .unwrap();
    let rejection = tokio::time::timeout(DEADLINE, async {
        loop {
            let response = recv_final(&socket).await;
            if response.contains(&call_id) {
                return response;
            }
        }
    })
    .await
    .expect("rejection deadline");
    assert!(rejection.starts_with("SIP/2.0 503"), "{rejection}");
    assert!(
        rejection.contains(OVERLOAD_TAG),
        "the 503 must come from the transaction layer: {rejection}"
    );
    assert_eq!(header(&rejection, "Retry-After"), Some("5"), "{rejection}");
    assert!(
        coordinator
            .sip_slot_cleanup_stats()
            .new_invite_capacity_rejections
            >= 1
    );

    coordinator.shutdown();
}
