//! Real localhost SIP MESSAGE <-> transport-neutral DataMessage coverage.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use rvoip_core::adapter::{AdapterEvent, ConnectionAdapter, EndReason, OriginateRequest};
use rvoip_core::capability::CapabilityDescriptor;
use rvoip_core::connection::{Direction, Transport};
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId};
use rvoip_core::{DataMessage, DataReliability, MessageId};
use rvoip_sip::api::unified::{Config as SipConfig, UnifiedCoordinator};
use rvoip_sip::auth::{DigestAuthenticator, SipClientAuth};
use rvoip_sip::{SipAdapter, SipOriginateContext};
use rvoip_sip_core::parser::parse_message;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderValue};
use rvoip_sip_dialog::transaction::utils::response_builders::create_response;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot};

const UAS_TAG: &str = "data-message-uas";

#[derive(Debug)]
struct EstablishedDialog {
    peer: std::net::SocketAddr,
    call_id: String,
    local_from: String,
    remote_to: String,
}

#[derive(Clone, Copy)]
enum ChallengeKind {
    Origin,
    Proxy,
}

impl ChallengeKind {
    fn challenge_status(self) -> StatusCode {
        match self {
            Self::Origin => StatusCode::Unauthorized,
            Self::Proxy => StatusCode::ProxyAuthenticationRequired,
        }
    }

    fn challenge_header(self) -> HeaderName {
        match self {
            Self::Origin => HeaderName::WwwAuthenticate,
            Self::Proxy => HeaderName::ProxyAuthenticate,
        }
    }

    fn credential_header(self) -> HeaderName {
        match self {
            Self::Origin => HeaderName::Authorization,
            Self::Proxy => HeaderName::ProxyAuthorization,
        }
    }
}

#[derive(Clone)]
enum MessageChallengePlan {
    None,
    Refresh { realm: String, nonce: String },
    WrongRealm { realm: String, nonce: String },
}

struct DialogAuthUas {
    addr: std::net::SocketAddr,
    requests: mpsc::UnboundedReceiver<Request>,
    task: tokio::task::JoinHandle<()>,
    _media_sink: UdpSocket,
}

impl Drop for DialogAuthUas {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn challenge_response(
    request: &Request,
    kind: ChallengeKind,
    realm: &str,
    nonce: &str,
    stale: bool,
) -> Response {
    let mut response = create_response(request, kind.challenge_status());
    if let Some(TypedHeader::To(to)) = response
        .headers
        .iter_mut()
        .find(|header| matches!(header, TypedHeader::To(_)))
    {
        to.set_tag(UAS_TAG);
    }
    let mut value =
        format!(r#"Digest realm="{realm}", nonce="{nonce}", algorithm=MD5, qop="auth""#);
    if stale {
        value.push_str(", stale=true");
    }
    response.headers.push(TypedHeader::Other(
        kind.challenge_header(),
        HeaderValue::Raw(value.into_bytes()),
    ));
    response
}

fn successful_invite_response(
    request: &Request,
    addr: std::net::SocketAddr,
    media_port: u16,
) -> Response {
    let mut response = create_response(request, StatusCode::Ok);
    if let Some(TypedHeader::To(to)) = response
        .headers
        .iter_mut()
        .find(|header| matches!(header, TypedHeader::To(_)))
    {
        to.set_tag(UAS_TAG);
    }
    response.headers.push(TypedHeader::Contact(
        Contact::from_str(&format!("<sip:data@{addr}>")).expect("auth UAS Contact"),
    ));
    response.headers.push(TypedHeader::ContentType(
        rvoip_sip_core::types::ContentType::sdp(),
    ));
    response.body = Bytes::from(format!(
        "v=0\r\no=data 1 1 IN IP4 127.0.0.1\r\ns=data-message-auth\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio {media_port} RTP/AVP 0 101\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:101 telephone-event/8000\r\na=fmtp:101 0-15\r\na=sendrecv\r\n"
    ));
    response
        .headers
        .retain(|header| !matches!(header, TypedHeader::ContentLength(_)));
    response.headers.push(TypedHeader::ContentLength(
        rvoip_sip_core::types::ContentLength::new(response.body.len() as u32),
    ));
    response
}

async fn boot_dialog_auth_uas(
    kind: ChallengeKind,
    invite_challenge: bool,
    realm: &str,
    old_nonce: &str,
    message_challenge: MessageChallengePlan,
) -> DialogAuthUas {
    let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("auth UAS bind"));
    let addr = socket.local_addr().expect("auth UAS address");
    let media_sink = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("auth media sink bind");
    let media_port = media_sink.local_addr().expect("auth media address").port();
    let (request_tx, request_rx) = mpsc::unbounded_channel();
    let realm = realm.to_string();
    let old_nonce = old_nonce.to_string();
    let task_socket = Arc::clone(&socket);
    let task = tokio::spawn(async move {
        let mut packet = vec![0u8; 65_536];
        loop {
            let (bytes, peer) = task_socket
                .recv_from(&mut packet)
                .await
                .expect("auth UAS receive");
            let Message::Request(request) =
                parse_message(&packet[..bytes]).expect("parse auth SIP request")
            else {
                continue;
            };
            match request.method() {
                Method::Invite => {
                    request_tx
                        .send(request.clone())
                        .expect("auth INVITE capture");
                    let response = if invite_challenge && digest_header(&request, kind).is_none() {
                        challenge_response(&request, kind, &realm, &old_nonce, false)
                    } else {
                        successful_invite_response(&request, addr, media_port)
                    };
                    task_socket
                        .send_to(&Message::Response(response).to_bytes(), peer)
                        .await
                        .expect("send auth INVITE response");
                }
                Method::Message => {
                    request_tx
                        .send(request.clone())
                        .expect("auth MESSAGE capture");
                    let response = match &message_challenge {
                        MessageChallengePlan::None => create_response(&request, StatusCode::Ok),
                        MessageChallengePlan::Refresh { realm, nonce } => {
                            let has_fresh_credential = digest_header(&request, kind)
                                .and_then(|value| {
                                    DigestAuthenticator::parse_authorization(&value).ok()
                                })
                                .is_some_and(|authorization| {
                                    authorization.realm == *realm && authorization.nonce == *nonce
                                });
                            if has_fresh_credential {
                                create_response(&request, StatusCode::Ok)
                            } else {
                                challenge_response(&request, kind, realm, nonce, true)
                            }
                        }
                        MessageChallengePlan::WrongRealm { realm, nonce } => {
                            challenge_response(&request, kind, realm, nonce, true)
                        }
                    };
                    task_socket
                        .send_to(&Message::Response(response).to_bytes(), peer)
                        .await
                        .expect("send auth MESSAGE response");
                }
                Method::Bye | Method::Cancel => {
                    request_tx
                        .send(request.clone())
                        .expect("auth teardown capture");
                    let response = create_response(&request, StatusCode::Ok);
                    task_socket
                        .send_to(&Message::Response(response).to_bytes(), peer)
                        .await
                        .expect("send auth teardown response");
                }
                Method::Ack => {}
                _ => {}
            }
        }
    });
    DialogAuthUas {
        addr,
        requests: request_rx,
        task,
        _media_sink: media_sink,
    }
}

fn digest_header(request: &Request, kind: ChallengeKind) -> Option<String> {
    request.raw_header_value(&kind.credential_header())
}

fn assert_digest_request(
    request: &Request,
    kind: ChallengeKind,
    method: &str,
    realm: &str,
    nonce: &str,
    password: &str,
) {
    let authorization = digest_header(request, kind).expect("method-specific Digest header");
    let parsed = DigestAuthenticator::parse_authorization(&authorization)
        .expect("parse method-specific Digest header");
    assert_eq!(parsed.realm, realm);
    assert_eq!(parsed.nonce, nonce);
    assert_eq!(parsed.uri, request.uri.to_string());
    assert!(
        DigestAuthenticator::new(realm)
            .validate_response_with_body(&parsed, method, password, Some(request.body.as_ref()))
            .expect("validate method-specific Digest response"),
        "Digest response did not bind the exact method, URI, and body"
    );
}

async fn next_method(requests: &mut mpsc::UnboundedReceiver<Request>, method: Method) -> Request {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let request = requests.recv().await.expect("auth UAS capture stream");
            if request.method() == method {
                return request;
            }
        }
    })
    .await
    .expect("auth UAS method deadline")
}

async fn next_method_with_auth(
    requests: &mut mpsc::UnboundedReceiver<Request>,
    method: Method,
    kind: ChallengeKind,
    present: bool,
) -> Request {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let request = requests.recv().await.expect("auth UAS capture stream");
            if request.method() == method && digest_header(&request, kind).is_some() == present {
                return request;
            }
        }
    })
    .await
    .expect("auth UAS method/auth deadline")
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

async fn originate_authenticated_dialog(
    adapter: &Arc<SipAdapter>,
    events: &mut mpsc::Receiver<AdapterEvent>,
    target: std::net::SocketAddr,
    realm: &str,
) -> ConnectionId {
    let credentials = rvoip_sip::types::Credentials::new("data-user", "data-password")
        .with_realm(realm.to_string());
    let context = SipOriginateContext::new()
        .with_auth(SipClientAuth::Digest(credentials))
        .expect("bounded exact-realm Digest context");
    let prepared = ConnectionAdapter::originate(
        adapter.as_ref(),
        OriginateRequest::new(
            SessionId::new(),
            ParticipantId::new(),
            format!("sip:data@{target}"),
            Direction::Outbound,
            CapabilityDescriptor::default(),
        )
        .with_transport(Transport::Sip)
        .with_context(context),
    )
    .await
    .expect("prepare authenticated dialog");
    let connection_id = prepared.connection.id.clone();
    ConnectionAdapter::activate_outbound_with_receipt(adapter.as_ref(), connection_id.clone())
        .await
        .expect("activate authenticated dialog");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Some(AdapterEvent::Connected { connection_id: id }) if id == connection_id => break,
                Some(_) => {}
                None => panic!("authenticated event stream closed before Connected"),
            }
        }
    })
    .await
    .expect("authenticated Connected deadline");
    connection_id
}

async fn run_stale_message_refresh(kind: ChallengeKind) {
    const REALM: &str = "dialog-data";
    const OLD_NONCE: &str = "dialog-old-nonce";
    const FRESH_NONCE: &str = "dialog-fresh-nonce";
    const PASSWORD: &str = "data-password";

    let mut uas = boot_dialog_auth_uas(
        kind,
        true,
        REALM,
        OLD_NONCE,
        MessageChallengePlan::Refresh {
            realm: REALM.to_string(),
            nonce: FRESH_NONCE.to_string(),
        },
    )
    .await;
    let coordinator = UnifiedCoordinator::new(SipConfig::local("dialog-data-uac", 0))
        .await
        .expect("authenticated coordinator");
    let adapter = SipAdapter::new(Arc::clone(&coordinator))
        .await
        .expect("authenticated adapter");
    let mut events = ConnectionAdapter::subscribe_events(adapter.as_ref());
    let connection_id =
        originate_authenticated_dialog(&adapter, &mut events, uas.addr, REALM).await;

    let initial_invite =
        next_method_with_auth(&mut uas.requests, Method::Invite, kind, false).await;
    assert!(digest_header(&initial_invite, kind).is_none());
    let authenticated_invite =
        next_method_with_auth(&mut uas.requests, Method::Invite, kind, true).await;
    assert_digest_request(
        &authenticated_invite,
        kind,
        "INVITE",
        REALM,
        OLD_NONCE,
        PASSWORD,
    );

    let message = DataMessage::reliable(
        "bridgefu.context.v1",
        "application/octet-stream",
        Bytes::from_static(b"method-uri-body-bound"),
    );
    ConnectionAdapter::send_data_message(adapter.as_ref(), connection_id.clone(), message)
        .await
        .expect("stale challenge refresh succeeds once");
    let preemptive = next_method(&mut uas.requests, Method::Message).await;
    let refreshed = next_method(&mut uas.requests, Method::Message).await;
    assert_digest_request(&preemptive, kind, "MESSAGE", REALM, OLD_NONCE, PASSWORD);
    assert_digest_request(&refreshed, kind, "MESSAGE", REALM, FRESH_NONCE, PASSWORD);
    assert_eq!(
        preemptive.call_id().map(|value| value.value()),
        refreshed.call_id().map(|value| value.value())
    );
    assert!(
        refreshed.cseq().unwrap().sequence() > preemptive.cseq().unwrap().sequence(),
        "bounded challenge retry must advance CSeq"
    );
    let refreshed_header = digest_header(&refreshed, kind).unwrap();
    let refreshed_auth = DigestAuthenticator::parse_authorization(&refreshed_header).unwrap();
    assert_eq!(refreshed_auth.nc.as_deref(), Some("00000001"));

    ConnectionAdapter::end(adapter.as_ref(), connection_id, EndReason::Normal)
        .await
        .expect("authenticated dialog end");
    let bye = next_method(&mut uas.requests, Method::Bye).await;
    assert_digest_request(&bye, kind, "BYE", REALM, FRESH_NONCE, PASSWORD);
    adapter.drain().await.expect("authenticated adapter drain");
    coordinator
        .shutdown_gracefully(Some(Duration::from_secs(1)))
        .await
        .expect("authenticated coordinator shutdown");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn data_message_origin_401_stale_nonce_refresh_is_bounded_and_method_specific() {
    run_stale_message_refresh(ChallengeKind::Origin).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn data_message_proxy_407_stale_nonce_refresh_is_bounded_and_method_specific() {
    run_stale_message_refresh(ChallengeKind::Proxy).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn data_message_wrong_realm_challenge_fails_closed_without_retry_or_secret_leak() {
    const REALM: &str = "dialog-data";
    const OLD_NONCE: &str = "dialog-old-nonce";
    const WRONG_NONCE: &str = "wrong-realm-nonce";
    const PASSWORD: &str = "data-password";

    let kind = ChallengeKind::Origin;
    let mut uas = boot_dialog_auth_uas(
        kind,
        true,
        REALM,
        OLD_NONCE,
        MessageChallengePlan::WrongRealm {
            realm: "other-protection-space".to_string(),
            nonce: WRONG_NONCE.to_string(),
        },
    )
    .await;
    let coordinator = UnifiedCoordinator::new(SipConfig::local("wrong-realm-uac", 0))
        .await
        .expect("wrong-realm coordinator");
    let adapter = SipAdapter::new(Arc::clone(&coordinator))
        .await
        .expect("wrong-realm adapter");
    let mut events = ConnectionAdapter::subscribe_events(adapter.as_ref());
    let connection_id =
        originate_authenticated_dialog(&adapter, &mut events, uas.addr, REALM).await;
    let _ = next_method_with_auth(&mut uas.requests, Method::Invite, kind, false).await;
    let _ = next_method_with_auth(&mut uas.requests, Method::Invite, kind, true).await;

    let result = ConnectionAdapter::send_data_message(
        adapter.as_ref(),
        connection_id.clone(),
        DataMessage::reliable(
            "bridgefu.context.v1",
            "application/octet-stream",
            Bytes::from_static(b"wrong-realm-must-fail"),
        ),
    )
    .await;
    let error = result.expect_err("wrong realm must fail closed");
    let rendered = format!("{error} {error:?}");
    for secret in [PASSWORD, WRONG_NONCE, "other-protection-space"] {
        assert!(
            !rendered.contains(secret),
            "credential detail leaked: {rendered}"
        );
    }
    let first_message = next_method(&mut uas.requests, Method::Message).await;
    assert_digest_request(&first_message, kind, "MESSAGE", REALM, OLD_NONCE, PASSWORD);
    assert!(
        tokio::time::timeout(Duration::from_millis(300), uas.requests.recv())
            .await
            .is_err(),
        "wrong-realm challenge was retried on the wire"
    );

    ConnectionAdapter::end(adapter.as_ref(), connection_id, EndReason::Normal)
        .await
        .expect("wrong-realm dialog end");
    adapter.drain().await.expect("wrong-realm adapter drain");
    coordinator
        .shutdown_gracefully(Some(Duration::from_secs(1)))
        .await
        .expect("wrong-realm coordinator shutdown");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn data_message_never_reuses_digest_credentials_across_dialogs() {
    const REALM: &str = "dialog-data";
    const NONCE: &str = "dialog-one-nonce";
    const PASSWORD: &str = "data-password";

    let kind = ChallengeKind::Origin;
    let mut protected =
        boot_dialog_auth_uas(kind, true, REALM, NONCE, MessageChallengePlan::None).await;
    let mut public =
        boot_dialog_auth_uas(kind, false, REALM, NONCE, MessageChallengePlan::None).await;
    let coordinator = UnifiedCoordinator::new(SipConfig::local("cross-dialog-uac", 0))
        .await
        .expect("cross-dialog coordinator");
    let adapter = SipAdapter::new(Arc::clone(&coordinator))
        .await
        .expect("cross-dialog adapter");
    let mut events = ConnectionAdapter::subscribe_events(adapter.as_ref());
    let protected_connection =
        originate_authenticated_dialog(&adapter, &mut events, protected.addr, REALM).await;
    let public_connection =
        originate_authenticated_dialog(&adapter, &mut events, public.addr, REALM).await;
    let _ = next_method_with_auth(&mut protected.requests, Method::Invite, kind, false).await;
    let protected_invite =
        next_method_with_auth(&mut protected.requests, Method::Invite, kind, true).await;
    assert_digest_request(&protected_invite, kind, "INVITE", REALM, NONCE, PASSWORD);
    let public_invite = next_method(&mut public.requests, Method::Invite).await;
    assert!(digest_header(&public_invite, kind).is_none());

    ConnectionAdapter::send_data_message(
        adapter.as_ref(),
        public_connection.clone(),
        DataMessage::reliable(
            "bridgefu.context.v1",
            "application/octet-stream",
            Bytes::from_static(b"public-dialog"),
        ),
    )
    .await
    .expect("public dialog MESSAGE");
    let public_message = next_method(&mut public.requests, Method::Message).await;
    assert!(
        public_message
            .raw_header_value(&HeaderName::Authorization)
            .is_none()
            && public_message
                .raw_header_value(&HeaderName::ProxyAuthorization)
                .is_none(),
        "another dialog's credential was attached to the public dialog"
    );

    ConnectionAdapter::send_data_message(
        adapter.as_ref(),
        protected_connection.clone(),
        DataMessage::reliable(
            "bridgefu.context.v1",
            "application/octet-stream",
            Bytes::from_static(b"protected-dialog"),
        ),
    )
    .await
    .expect("protected dialog MESSAGE");
    let protected_message = next_method(&mut protected.requests, Method::Message).await;
    assert_digest_request(&protected_message, kind, "MESSAGE", REALM, NONCE, PASSWORD);

    ConnectionAdapter::end(adapter.as_ref(), public_connection, EndReason::Normal)
        .await
        .expect("public dialog end");
    ConnectionAdapter::end(adapter.as_ref(), protected_connection, EndReason::Normal)
        .await
        .expect("protected dialog end");
    adapter.drain().await.expect("cross-dialog adapter drain");
    coordinator
        .shutdown_gracefully(Some(Duration::from_secs(1)))
        .await
        .expect("cross-dialog coordinator shutdown");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn data_message_round_trips_over_real_in_dialog_sip_message() {
    let uas = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("UAS bind"));
    let uas_addr = uas.local_addr().expect("UAS address");
    let media_sink = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("media sink bind");
    let media_port = media_sink.local_addr().expect("media address").port();
    let (established_tx, established_rx) = oneshot::channel();
    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel();
    let (inbound_response_tx, inbound_response_rx) = oneshot::channel();
    let task_socket = Arc::clone(&uas);
    let uas_task = tokio::spawn(async move {
        let mut packet = vec![0u8; 65_536];
        let mut established_tx = Some(established_tx);
        let mut inbound_response_tx = Some(inbound_response_tx);
        loop {
            let (bytes, peer) = task_socket
                .recv_from(&mut packet)
                .await
                .expect("UAS receive");
            match parse_message(&packet[..bytes]).expect("parse SIP message") {
                Message::Request(request) => match request.method() {
                    Method::Invite => {
                        let mut response = create_response(&request, StatusCode::Ok);
                        if let Some(TypedHeader::To(to)) = response
                            .headers
                            .iter_mut()
                            .find(|header| matches!(header, TypedHeader::To(_)))
                        {
                            to.set_tag(UAS_TAG);
                        }
                        response.headers.push(TypedHeader::Contact(
                            Contact::from_str(&format!("<sip:data@{uas_addr}>"))
                                .expect("UAS Contact"),
                        ));
                        response.headers.push(TypedHeader::ContentType(
                            rvoip_sip_core::types::ContentType::sdp(),
                        ));
                        response.body = Bytes::from(format!(
                            "v=0\r\no=data 1 1 IN IP4 127.0.0.1\r\ns=data-message\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio {media_port} RTP/AVP 0 101\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:101 telephone-event/8000\r\na=fmtp:101 0-15\r\na=sendrecv\r\n"
                        ));
                        response
                            .headers
                            .retain(|header| !matches!(header, TypedHeader::ContentLength(_)));
                        response.headers.push(TypedHeader::ContentLength(
                            rvoip_sip_core::types::ContentLength::new(response.body.len() as u32),
                        ));
                        task_socket
                            .send_to(&Message::Response(response).to_bytes(), peer)
                            .await
                            .expect("send INVITE response");
                    }
                    Method::Ack => {
                        if let Some(sender) = established_tx.take() {
                            sender
                                .send(EstablishedDialog {
                                    peer,
                                    call_id: request
                                        .call_id()
                                        .map(|value| value.value())
                                        .expect("ACK Call-ID"),
                                    local_from: request
                                        .raw_header_value(&HeaderName::From)
                                        .expect("ACK From"),
                                    remote_to: request
                                        .raw_header_value(&HeaderName::To)
                                        .expect("ACK To"),
                                })
                                .expect("established receiver");
                        }
                    }
                    Method::Message => {
                        outbound_tx
                            .send(request.clone())
                            .expect("outbound MESSAGE receiver");
                        let response = create_response(&request, StatusCode::Ok);
                        task_socket
                            .send_to(&Message::Response(response).to_bytes(), peer)
                            .await
                            .expect("send MESSAGE response");
                    }
                    Method::Bye | Method::Cancel => {
                        let response = create_response(&request, StatusCode::Ok);
                        task_socket
                            .send_to(&Message::Response(response).to_bytes(), peer)
                            .await
                            .expect("send teardown response");
                    }
                    _ => {}
                },
                Message::Response(response) => {
                    if response
                        .cseq()
                        .is_some_and(|cseq| cseq.method == Method::Message)
                    {
                        if let Some(sender) = inbound_response_tx.take() {
                            let _ = sender.send(response.status().as_u16());
                        }
                    }
                }
            }
        }
    });

    let coordinator = UnifiedCoordinator::new(SipConfig::local("data-message-uac", 0))
        .await
        .expect("coordinator");
    let adapter = SipAdapter::new(Arc::clone(&coordinator))
        .await
        .expect("adapter");
    let mut events = ConnectionAdapter::subscribe_events(adapter.as_ref());
    let mut sip_events = coordinator.events().await.expect("SIP event receiver");
    let prepared = ConnectionAdapter::originate(
        adapter.as_ref(),
        OriginateRequest::new(
            SessionId::new(),
            ParticipantId::new(),
            format!("sip:data@{uas_addr}"),
            Direction::Outbound,
            CapabilityDescriptor::default(),
        )
        .with_transport(Transport::Sip),
    )
    .await
    .expect("prepare route");
    let connection_id = prepared.connection.id.clone();
    ConnectionAdapter::activate_outbound_with_receipt(adapter.as_ref(), connection_id.clone())
        .await
        .expect("activate route");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Some(AdapterEvent::Connected { connection_id: id }) if id == connection_id => break,
                Some(_) => {}
                None => panic!("event stream closed before Connected"),
            }
        }
    })
    .await
    .expect("Connected deadline");
    let established = tokio::time::timeout(Duration::from_secs(5), established_rx)
        .await
        .expect("ACK deadline")
        .expect("ACK dialog");

    let outbound = DataMessage::try_new(
        "bridgefu.context.v1",
        "application/octet-stream",
        Bytes::from_static(&[0, 0xff, b'\r', b'\n', 0x80, 1]),
        DataReliability::ReliableOrdered,
        MessageId::from_string("network-outbound-1"),
    )
    .expect("outbound DataMessage");
    ConnectionAdapter::send_data_message(adapter.as_ref(), connection_id.clone(), outbound.clone())
        .await
        .expect("send outbound DataMessage");
    let captured = tokio::time::timeout(Duration::from_secs(5), outbound_rx.recv())
        .await
        .expect("outbound MESSAGE deadline")
        .expect("outbound MESSAGE");
    assert_eq!(captured.body, outbound.bytes);
    assert_eq!(
        application_header(&captured, "X-Bridgefu-Data-Label").as_deref(),
        Some(outbound.label.as_bytes())
    );
    assert_eq!(
        application_header(&captured, "X-Bridgefu-Data-Content-Type").as_deref(),
        Some(outbound.content_type.as_bytes())
    );
    assert_eq!(
        application_header(&captured, "X-Bridgefu-Message-Id").as_deref(),
        Some(outbound.message_id.as_str().as_bytes())
    );
    assert_eq!(
        application_header(&captured, "X-Bridgefu-Data-Reliability").as_deref(),
        Some(b"reliable-ordered".as_slice())
    );

    let inbound = DataMessage::try_new(
        "bridgefu.context.v1",
        "application/octet-stream",
        Bytes::from_static(&[9, 0, 0xfe, b'\r', b'\n', 7]),
        DataReliability::ReliableOrdered,
        MessageId::from_string("network-inbound-1"),
    )
    .expect("inbound DataMessage");
    let headers = format!(
        "MESSAGE sip:data-uac@{} SIP/2.0\r\n\
         Via: SIP/2.0/UDP {uas_addr};branch=z9hG4bK-data-inbound;rport\r\n\
         From: {}\r\n\
         To: {}\r\n\
         Call-ID: {}\r\n\
         CSeq: 2 MESSAGE\r\n\
         Max-Forwards: 70\r\n\
         Contact: <sip:data@{uas_addr}>\r\n\
         Content-Type: {}\r\n\
         X-Bridgefu-Data-Label: {}\r\n\
         X-Bridgefu-Data-Content-Type: {}\r\n\
         X-Bridgefu-Message-Id: {}\r\n\
         X-Bridgefu-Data-Reliability: reliable-ordered\r\n\
         Content-Length: {}\r\n\r\n",
        established.peer,
        established.remote_to,
        established.local_from,
        established.call_id,
        inbound.content_type,
        inbound.label,
        inbound.content_type,
        inbound.message_id.as_str(),
        inbound.bytes.len(),
    );
    let mut wire = headers.into_bytes();
    wire.extend_from_slice(&inbound.bytes);
    uas.send_to(&wire, established.peer)
        .await
        .expect("send inbound MESSAGE");

    let inbound_status = tokio::time::timeout(Duration::from_secs(5), inbound_response_rx)
        .await
        .expect("inbound MESSAGE response deadline")
        .expect("inbound MESSAGE response");
    assert_eq!(inbound_status, 200, "in-dialog MESSAGE must be accepted");

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match sip_events.next().await {
                Some(rvoip_sip::Event::MessageReceived { .. }) => break,
                Some(_) => {}
                None => panic!("SIP event stream closed before MessageReceived"),
            }
        }
    })
    .await
    .expect("SIP MessageReceived deadline");

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Some(AdapterEvent::DataMessage {
                    connection_id: id,
                    message,
                }) if id == connection_id => {
                    assert_eq!(message, inbound);
                    break;
                }
                Some(_) => {}
                None => panic!("event stream closed before inbound DataMessage"),
            }
        }
    })
    .await
    .expect("inbound DataMessage deadline");

    ConnectionAdapter::end(adapter.as_ref(), connection_id, EndReason::Normal)
        .await
        .expect("end route");
    adapter.drain().await.expect("adapter drain");
    coordinator
        .shutdown_gracefully(Some(Duration::from_secs(1)))
        .await
        .expect("coordinator shutdown");
    uas_task.abort();
}
