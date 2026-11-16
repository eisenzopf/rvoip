//! `Config::sip_tls_proxy_flow_policies` on real certificate-verified TLS
//! sockets.
//!
//! The test plays a carrier's record-routing TLS proxy. It opens a TLS
//! connection to the endpoint, delivers an INVITE whose Record-Route names
//! the proxy's logical listener, and keeps that listener address bound but
//! not listening. A normal in-dialog request therefore fails to connect,
//! and a request that reaches the test can only have used the accepted
//! connection it arrived on.
use rvoip_core_traits::identity::{
    AuthenticatedPrincipal, AuthenticationMethod, IdentityAssurance,
};
use rvoip_sip::api::events::Event;
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::unified::{Config, SipTlsMode, SipTlsProxyFlowPolicy, UnifiedCoordinator};
use rvoip_sip::SipListenerAuthPolicy;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const DEADLINE: Duration = Duration::from_secs(10);
const ISSUER: &str = "trusted-cidr";
const TENANT: &str = "edge";
const SUBJECT: &str = "carrier-proxy";
const REMOTE_TARGET: &str = "sip:carrier@10.0.0.9:6000";

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
            key.trim()
                .eq_ignore_ascii_case(name)
                .then_some(value.trim())
        })
        .unwrap_or_else(|| panic!("missing {name} header"))
}

/// One SIP-over-TLS connection as the proxy sees it, with message framing.
struct ProxyWire<S>(S, Vec<u8>);

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> ProxyWire<S> {
    async fn send(&mut self, message: &str) {
        self.0.write_all(message.as_bytes()).await.unwrap();
    }

    /// Next complete message containing `expected`; earlier ones are skipped.
    async fn until(&mut self, expected: &str) -> String {
        tokio::time::timeout(DEADLINE, async {
            loop {
                while let Some(end) = self.1.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let head = std::str::from_utf8(&self.1[..end]).unwrap();
                    let length: usize = header(head, "Content-Length").parse().unwrap();
                    let total = end + 4 + length;
                    if self.1.len() < total {
                        break;
                    }
                    let message = String::from_utf8(self.1.drain(..total).collect()).unwrap();
                    if message.contains(expected) {
                        return message;
                    }
                }
                let mut bytes = vec![0; 16_384];
                let count = self.0.read(&mut bytes).await.unwrap();
                assert!(count > 0, "TLS flow closed while awaiting {expected}");
                self.1.extend_from_slice(&bytes[..count]);
            }
        })
        .await
        .unwrap_or_else(|_| panic!("no {expected} on this TLS flow"))
    }

    /// Fail if a request starting with `method` arrives within `window`.
    async fn assert_no_request(&mut self, method: &str, window: Duration) {
        let mut bytes = [0; 8192];
        if let Ok(Ok(count)) = tokio::time::timeout(window, self.0.read(&mut bytes)).await {
            let text = String::from_utf8_lossy(&bytes[..count]);
            assert!(
                !text.contains(&format!("{method} sip:")),
                "{method} was sent on a flow it must not use"
            );
        }
    }

    async fn receive_bye(&mut self, call_id: &str, route: &str) -> String {
        let bye = self.until("BYE ").await;
        assert!(bye.starts_with(&format!("BYE {REMOTE_TARGET} SIP/2.0")));
        assert_eq!(header(&bye, "Call-ID"), call_id);
        assert_eq!(header(&bye, "Route"), route);
        assert!(header(&bye, "Via").starts_with("SIP/2.0/TLS "));
        bye
    }

    async fn respond(&mut self, request: &str, status: &str) {
        self.send(&format!(
            "SIP/2.0 {status}\r\nVia: {}\r\nFrom: {}\r\nTo: {}\r\nCall-ID: {}\r\nCSeq: {}\r\nContent-Length: 0\r\n\r\n",
            header(request, "Via"),
            header(request, "From"),
            header(request, "To"),
            header(request, "Call-ID"),
            header(request, "CSeq"),
        ))
        .await;
    }
}

type ClientWire = ProxyWire<tokio_rustls::client::TlsStream<TcpStream>>;

struct Fixture {
    endpoint: Arc<UnifiedCoordinator>,
    tls_port: u16,
    /// The proxy's logical listener: bound, and not listening unless a test
    /// turns it into a listener.
    proxy: SocketAddr,
    proxy_reservation: Option<tokio::net::TcpSocket>,
    certificate: rustls::pki_types::CertificateDer<'static>,
    acceptor: tokio_rustls::TlsAcceptor,
    _directory: tempfile::TempDir,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Case {
    Enabled,
    Renewal,
    Disabled,
    WrongSubject,
    WrongTenant,
    WrongIssuer,
    WrongSource,
}

impl Fixture {
    async fn new(case: Case) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let directory = tempfile::tempdir().unwrap();
        let identity = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let cert = directory.path().join("cert.pem");
        let key = directory.path().join("key.pem");
        std::fs::write(&cert, identity.cert.pem()).unwrap();
        std::fs::write(&key, identity.signing_key.serialize_pem()).unwrap();
        let tls_port = port();
        let proxy_reservation = tokio::net::TcpSocket::new_v4().unwrap();
        proxy_reservation.bind(([127, 0, 0, 1], 0).into()).unwrap();
        let proxy = proxy_reservation.local_addr().unwrap();

        let mut config = Config::local("gateway", port());
        config.sip_tls_mode = SipTlsMode::ClientAndServer;
        config.tls_bind_addr = Some(([127, 0, 0, 1], tls_port).into());
        config.tls_advertised_addr = config.tls_bind_addr;
        config.sip_advertised_addr = config.tls_bind_addr;
        config.tls_cert_path = Some(cert.clone());
        config.tls_key_path = Some(key);
        // Renewal dials the proxy's fixed listener and verifies its
        // certificate, which this self-signed CA issued for "localhost".
        config.tls_extra_ca_path = Some(cert);
        let mut policy = SipTlsProxyFlowPolicy::new(ISSUER, TENANT, SUBJECT, proxy);
        match case {
            Case::Renewal => policy = policy.with_renewal_authority("localhost"),
            Case::WrongSubject => policy.subject = "another-proxy".into(),
            Case::WrongTenant => policy.tenant = "another-edge".into(),
            Case::WrongSource => policy.proxy_endpoint.set_ip("127.0.0.2".parse().unwrap()),
            _ => {}
        }
        if case != Case::Disabled {
            config = config.with_sip_tls_proxy_flow_policies([policy]);
        }
        config.validate().unwrap();

        let principal = AuthenticatedPrincipal {
            subject: SUBJECT.into(),
            tenant: Some(TENANT.into()),
            scopes: vec!["call:attach".into()],
            issuer: Some(
                if case == Case::WrongIssuer {
                    "another-issuer"
                } else {
                    ISSUER
                }
                .into(),
            ),
            expires_at: None,
            method: AuthenticationMethod::ApiKey,
            assurance: IdentityAssurance::Anonymous,
        };
        let auth = SipListenerAuthPolicy::enabled_for_tenant(TENANT)
            .unwrap()
            .with_trusted_cidr("127.0.0.1/32".parse().unwrap(), principal);
        let endpoint = Box::pin(UnifiedCoordinator::new_with_listener_auth(config, auth))
            .await
            .unwrap();
        let server = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![identity.cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der())
                .into(),
        )
        .unwrap();
        Self {
            endpoint,
            tls_port,
            proxy,
            proxy_reservation: Some(proxy_reservation),
            certificate: identity.cert.der().clone(),
            acceptor: tokio_rustls::TlsAcceptor::from(Arc::new(server)),
            _directory: directory,
        }
    }

    fn route(&self) -> String {
        format!("<sip:{};transport=tls;lr>", self.proxy)
    }

    /// Start listening on the proxy's logical address without releasing it.
    fn listen_on_proxy_address(&mut self) -> tokio::net::TcpListener {
        self.proxy_reservation.take().unwrap().listen(1).unwrap()
    }

    async fn connect(&self) -> ClientWire {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(self.certificate.clone()).unwrap();
        let client = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
        let stream = TcpStream::connect(("127.0.0.1", self.tls_port))
            .await
            .unwrap();
        ProxyWire(
            tokio_rustls::TlsConnector::from(Arc::new(client))
                .connect(
                    rustls::pki_types::ServerName::try_from("localhost").unwrap(),
                    stream,
                )
                .await
                .unwrap(),
            Vec::new(),
        )
    }

    /// Deliver and answer an INVITE through the proxy on a new TLS flow.
    async fn establish(
        &self,
        events: &mut EventReceiver,
        call_id: &str,
    ) -> (ClientWire, rvoip_sip::SessionId) {
        let mut wire = self.connect().await;
        let sdp = sdp(1, "sendrecv");
        let route = self.route();
        wire.send(&format!(
            "INVITE sip:gateway@127.0.0.1:{port} SIP/2.0\r\nVia: SIP/2.0/TLS {proxy};branch=z9hG4bK-{call_id};rport\r\nRecord-Route: {route}\r\nMax-Forwards: 70\r\nFrom: <sip:carrier@example.test>;tag={call_id}\r\nTo: <sip:gateway@127.0.0.1:{port}>\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <{REMOTE_TARGET}>\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{sdp}",
            sdp.len(),
            port = self.tls_port,
            proxy = self.proxy,
        ))
        .await;
        let session = tokio::time::timeout(DEADLINE, async {
            loop {
                if let Event::IncomingCall { call_id, .. } = events.next().await.unwrap() {
                    break call_id;
                }
            }
        })
        .await
        .expect("trusted INVITE reaches the application");
        self.endpoint.accept_call(&session).await.unwrap();
        let response = wire.until("SIP/2.0 200 ").await;
        wire.send(&format!(
            "ACK {} SIP/2.0\r\nVia: SIP/2.0/TLS {};branch=z9hG4bK-ack-{call_id};rport\r\nMax-Forwards: 70\r\nFrom: <sip:carrier@example.test>;tag={call_id}\r\nTo: {}\r\nCall-ID: {call_id}\r\nCSeq: 1 ACK\r\nContent-Length: 0\r\n\r\n",
            header(&response, "Contact").trim_matches(['<', '>']),
            self.proxy,
            header(&response, "To"),
        ))
        .await;
        tokio::time::timeout(DEADLINE, async {
            loop {
                if matches!(
                    events.next().await.unwrap(),
                    Event::CallEstablished { call_id: ref established, .. } if established == &session
                ) {
                    break;
                }
            }
        })
        .await
        .expect("ACK establishes the call");
        (wire, session)
    }
}

fn sdp(version: u32, direction: &str) -> String {
    format!(
        "v=0\r\no=carrier 1 {version} IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 45000 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\na={direction}\r\na=ptime:20\r\n"
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn policy_sends_each_dialog_s_bye_on_its_own_accepted_tls_flow() {
    let fixture = Fixture::new(Case::Enabled).await;
    let mut events = fixture.endpoint.events().await.unwrap();
    let route = fixture.route();
    let (mut first, first_id) = fixture.establish(&mut events, "proxy-flow-first").await;
    let (mut second, second_id) = fixture.establish(&mut events, "proxy-flow-second").await;
    // The proxy's logical listener refuses connections, so each BYE can
    // only arrive on the flow its own dialog was delivered on.
    let (first_result, second_result, (), ()) = tokio::time::timeout(DEADLINE, async {
        tokio::join!(
            fixture.endpoint.hangup(&first_id),
            fixture.endpoint.hangup(&second_id),
            async {
                let bye = first.receive_bye("proxy-flow-first", &route).await;
                first.respond(&bye, "200 OK").await;
            },
            async {
                let bye = second.receive_bye("proxy-flow-second", &route).await;
                second.respond(&bye, "200 OK").await;
            }
        )
    })
    .await
    .expect("both BYEs confirmed on their original flows");
    first_result.unwrap();
    second_result.unwrap();
    fixture.endpoint.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn policy_sends_hold_reinvite_on_the_accepted_tls_flow() {
    let fixture = Fixture::new(Case::Enabled).await;
    let mut events = fixture.endpoint.events().await.unwrap();
    let route = fixture.route();
    let (mut wire, session) = fixture.establish(&mut events, "proxy-flow-hold").await;
    // The re-INVITE arrives on the accepted flow. It is declined so that its
    // ACK is the INVITE transaction's own hop-by-hop ACK, which must follow
    // the same flow; the end-to-end ACK of a 2xx re-INVITE is outside this
    // feature.
    let (_hold, ()) = tokio::time::timeout(DEADLINE, async {
        tokio::join!(fixture.endpoint.hold(&session), async {
            let invite = wire.until("INVITE ").await;
            assert!(invite.starts_with(&format!("INVITE {REMOTE_TARGET} SIP/2.0")));
            assert_eq!(header(&invite, "Call-ID"), "proxy-flow-hold");
            assert_eq!(header(&invite, "Route"), route);
            assert!(header(&invite, "Via").starts_with("SIP/2.0/TLS "));
            assert!(invite.contains("a=sendonly"));
            wire.respond(&invite, "488 Not Acceptable Here").await;
            let ack = wire.until("ACK ").await;
            assert_eq!(header(&ack, "Call-ID"), "proxy-flow-hold");
            assert_eq!(
                header(&ack, "CSeq").split_whitespace().next(),
                header(&invite, "CSeq").split_whitespace().next()
            );
        })
    })
    .await
    .expect("hold re-INVITE and its ACK use the accepted flow");
    fixture.endpoint.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bye_does_not_use_the_accepted_flow_without_a_matching_policy() {
    for case in [
        Case::Disabled,
        Case::WrongSubject,
        Case::WrongTenant,
        Case::WrongIssuer,
        Case::WrongSource,
    ] {
        let fixture = Fixture::new(case).await;
        let mut events = fixture.endpoint.events().await.unwrap();
        let (mut wire, session) = fixture
            .establish(&mut events, &format!("no-reuse-{case:?}"))
            .await;
        // Normal routing dials the proxy's logical listener, which refuses.
        let result = tokio::time::timeout(DEADLINE, fixture.endpoint.hangup(&session))
            .await
            .unwrap_or_else(|_| panic!("{case:?}: hangup must settle promptly"));
        assert!(
            result.is_err(),
            "{case:?}: the BYE cannot be delivered without reusing the accepted flow"
        );
        wire.assert_no_request("BYE", Duration::from_millis(300))
            .await;
        fixture.endpoint.shutdown();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn closed_flow_is_not_replaced_by_a_new_connection_without_renewal() {
    let mut fixture = Fixture::new(Case::Enabled).await;
    let mut events = fixture.endpoint.events().await.unwrap();
    let (mut wire, session) = fixture.establish(&mut events, "closed-proxy-flow").await;
    let listener = fixture.listen_on_proxy_address();
    wire.0.shutdown().await.unwrap();
    drop(wire);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let result = tokio::time::timeout(DEADLINE, fixture.endpoint.hangup(&session))
        .await
        .expect("closed-flow hangup settles promptly");
    assert!(result.is_err(), "a closed flow cannot carry the BYE");
    assert!(
        tokio::time::timeout(Duration::from_millis(500), listener.accept())
            .await
            .is_err(),
        "a closed bound flow must not trigger a new TLS connection"
    );
    fixture.endpoint.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn renewal_authority_replaces_a_closed_flow_with_a_verified_connection_to_the_proxy() {
    let mut fixture = Fixture::new(Case::Renewal).await;
    let mut events = fixture.endpoint.events().await.unwrap();
    let route = fixture.route();
    let (mut original, session) = fixture.establish(&mut events, "renewed-proxy-flow").await;
    let listener = fixture.listen_on_proxy_address();
    original.0.shutdown().await.unwrap();
    drop(original);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let acceptor = fixture.acceptor.clone();
    let (result, ()) = tokio::time::timeout(DEADLINE, async {
        tokio::join!(fixture.endpoint.hangup(&session), async {
            let (socket, _) = listener.accept().await.unwrap();
            let tls = acceptor.accept(socket).await.unwrap();
            let mut renewed = ProxyWire(tls, Vec::new());
            let bye = renewed.receive_bye("renewed-proxy-flow", &route).await;
            renewed.respond(&bye, "200 OK").await;
            // Keep the connection open until the response is consumed.
            renewed
                .assert_no_request("BYE", Duration::from_millis(300))
                .await;
        })
    })
    .await
    .expect("renewed flow carries the BYE");
    result.unwrap();
    fixture.endpoint.shutdown();
}

/// A carrier dialled by DNS name whose 2xx record-routes with its IP
/// address, which its certificate does not name. Returns whether our BYE
/// arrived on the connection the INVITE used.
async fn outbound_bye_reuses_invite_connection(retain: bool) -> bool {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let directory = tempfile::tempdir().unwrap();
    let identity = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert = directory.path().join("cert.pem");
    std::fs::write(&cert, identity.cert.pem()).unwrap();
    let server = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![identity.cert.der().clone()],
        rustls::pki_types::PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der()).into(),
    )
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let carrier = listener.local_addr().unwrap();

    let key = directory.path().join("key.pem");
    std::fs::write(&key, identity.signing_key.serialize_pem()).unwrap();
    let mut config = Config::local("gateway", port());
    config.sip_tls_mode = SipTlsMode::ClientAndServer;
    config.tls_bind_addr = Some(([127, 0, 0, 1], port()).into());
    config.tls_advertised_addr = config.tls_bind_addr;
    config.sip_advertised_addr = config.tls_bind_addr;
    config.tls_cert_path = Some(cert.clone());
    config.tls_key_path = Some(key);
    config.tls_extra_ca_path = Some(cert);
    let config = config.with_sip_retain_outbound_tls_flow(retain);
    let endpoint = Box::pin(UnifiedCoordinator::new(config)).await.unwrap();

    let target = format!("sips:carrier@localhost:{}", carrier.port());
    let (call, mut wire) = tokio::time::timeout(DEADLINE, async {
        tokio::join!(endpoint.invite(None, target).send(), async {
            let (socket, _) = listener.accept().await.unwrap();
            let mut wire = ProxyWire(acceptor.accept(socket).await.unwrap(), Vec::new());
            let invite = wire.until("INVITE ").await;
            let answer = sdp(1, "sendrecv");
            wire.send(&format!(
                "SIP/2.0 200 OK\r\nVia: {}\r\nRecord-Route: <sip:{carrier};transport=tls;lr>\r\nFrom: {}\r\nTo: {};tag=carrier\r\nCall-ID: {}\r\nCSeq: {}\r\nContact: <sips:carrier@{carrier}>\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{answer}",
                header(&invite, "Via"),
                header(&invite, "From"),
                header(&invite, "To"),
                header(&invite, "Call-ID"),
                header(&invite, "CSeq"),
                answer.len()
            ))
            .await;
            wire.until("ACK ").await;
            wire
        })
    })
    .await
    .expect("outbound TLS call is answered and acknowledged");
    let call = call.unwrap();
    // Without retention the BYE dials the IP literal and fails certificate
    // verification, so it can only arrive here on the INVITE's connection.
    let route = format!("<sip:{carrier};transport=tls;lr>");
    let (hangup, received) = tokio::join!(
        tokio::time::timeout(DEADLINE, endpoint.hangup(&call)),
        tokio::time::timeout(Duration::from_secs(3), async {
            let bye = wire.until("BYE ").await;
            assert_eq!(header(&bye, "Route"), route);
            wire.respond(&bye, "200 OK").await;
        })
    );
    let received = received.is_ok();
    if received {
        hangup.expect("hangup settles").unwrap();
    }
    endpoint.shutdown();
    received
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn outbound_retention_keeps_bye_on_the_verified_invite_connection() {
    assert!(outbound_bye_reuses_invite_connection(true).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn outbound_bye_is_routed_normally_without_retention() {
    assert!(!outbound_bye_reuses_invite_connection(false).await);
}
