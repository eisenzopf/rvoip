//! `Config::sip_tls_flow` reaches the SIP TLS listener, and
//! `Config::with_sip_bind_outbound_to_local_ip` sets the TCP and TLS flows.
use rvoip_sip::api::unified::{Config, SipTlsMode};
use rvoip_sip_core::builder::SimpleResponseBuilder;
use rvoip_sip_core::{Message, StatusCode};
use rvoip_sip_transport::{
    IngressAdmissionHook, IngressVerdict, TcpFlowConfig, TlsFlowConfig,
    TlsReceiveBackpressurePolicy,
};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const DEADLINE: Duration = Duration::from_secs(10);

#[path = "support/free_port.rs"]
mod free_port;

use free_port::{probe_udp_port, start_with_tls_connection};

#[test]
fn sip_tls_flow_defaults_to_pause_with_nodelay_and_is_configurable() {
    let config = Config::local("alice", 5060);
    assert_eq!(
        config.sip_tls_flow.receive_backpressure,
        TlsReceiveBackpressurePolicy::Pause
    );
    assert_eq!(
        config.sip_tls_flow.receive_pause_max,
        Duration::from_secs(600)
    );
    assert!(config.sip_tls_flow.tcp_nodelay);
    assert!(config.sip_tls_flow.ingress_admission.is_none());

    let tuned = Config::local("alice", 5060).with_sip_tls_flow(
        TlsFlowConfig::default()
            .with_receive_backpressure(TlsReceiveBackpressurePolicy::Close)
            .with_tcp_nodelay(false),
    );
    assert_eq!(
        tuned.sip_tls_flow.receive_backpressure,
        TlsReceiveBackpressurePolicy::Close
    );
    assert!(!tuned.sip_tls_flow.tcp_nodelay);
    assert!(format!("{tuned:?}").contains("sip_tls_flow"));
}

#[test]
fn outbound_source_bind_is_off_by_default_and_set_for_tcp_and_tls_together() {
    let config = Config::local("alice", 5060);
    assert!(!config.sip_tls_flow.bind_outbound_to_local_ip);
    assert!(!config.sip_tcp_flow.bind_outbound_to_local_ip);

    let bound = Config::local("alice", 5060)
        .with_sip_tls_flow(TlsFlowConfig::default().with_tcp_nodelay(false))
        .with_sip_bind_outbound_to_local_ip(true);
    assert!(bound.sip_tls_flow.bind_outbound_to_local_ip);
    assert!(bound.sip_tcp_flow.bind_outbound_to_local_ip);
    // The convenience setter keeps the flows' other settings.
    assert!(!bound.sip_tls_flow.tcp_nodelay);
    assert!(format!("{bound:?}").contains("sip_tcp_flow"));

    let cleared = bound.with_sip_bind_outbound_to_local_ip(false);
    assert!(!cleared.sip_tls_flow.bind_outbound_to_local_ip);
    assert!(!cleared.sip_tcp_flow.bind_outbound_to_local_ip);

    let tcp_only = Config::local("alice", 5060)
        .with_sip_tcp_flow(TcpFlowConfig::default().with_bind_outbound_to_local_ip(true));
    assert!(tcp_only.sip_tcp_flow.bind_outbound_to_local_ip);
    assert!(!tcp_only.sip_tls_flow.bind_outbound_to_local_ip);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sip_tls_flow_ingress_hook_answers_on_the_tls_listener() {
    let directory = tempfile::tempdir().unwrap();
    let identity = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert = directory.path().join("cert.pem");
    let key = directory.path().join("key.pem");
    std::fs::write(&cert, identity.cert.pem()).unwrap();
    std::fs::write(&key, identity.signing_key.serialize_pem()).unwrap();

    // Answer probes statelessly before they reach the transaction layer.
    let hook = IngressAdmissionHook::new(|message, _source| match message {
        Message::Request(request)
            if request
                .call_id()
                .is_some_and(|call_id| call_id.as_str().starts_with("tls-flow-")) =>
        {
            let response = SimpleResponseBuilder::response_from_request(
                request,
                StatusCode::ServiceUnavailable,
                Some("Overloaded"),
            )
            .build();
            IngressVerdict::Respond(Message::Response(response).to_bytes().into())
        }
        _ => IngressVerdict::Pass,
    });
    let flow = TlsFlowConfig::default().with_ingress_admission(hook);

    let (coordinator, mut stream, tls_port) =
        start_with_tls_connection("TLS appliance", identity.cert.der().clone(), |tls_port| {
            let mut config = Config::local("appliance", probe_udp_port());
            config.sip_tls_mode = SipTlsMode::ClientAndServer;
            config.tls_bind_addr = Some(([127, 0, 0, 1], tls_port).into());
            config.tls_cert_path = Some(cert.clone());
            config.tls_key_path = Some(key.clone());
            config.with_sip_tls_flow(flow.clone())
        })
        .await;

    let call_id = format!("tls-flow-{tls_port}");
    let options = format!(
        "OPTIONS sips:appliance@127.0.0.1:{tls_port} SIP/2.0\r\nVia: SIP/2.0/TLS 127.0.0.1:5061;branch=z9hG4bK-{call_id}\r\nMax-Forwards: 70\r\nFrom: <sips:probe@127.0.0.1>;tag=probe\r\nTo: <sips:appliance@127.0.0.1:{tls_port}>\r\nCall-ID: {call_id}\r\nCSeq: 1 OPTIONS\r\nContent-Length: 0\r\n\r\n"
    );
    stream.write_all(options.as_bytes()).await.unwrap();

    let response = tokio::time::timeout(DEADLINE, async {
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
    .expect("hook response deadline");
    assert!(response.starts_with("SIP/2.0 503"), "{response}");
    assert!(response.contains(&call_id), "{response}");

    let stats = flow.stats();
    assert_eq!(stats.ingress_responded, 1);
    assert_eq!(stats.ingress_responses_dropped, 0);

    coordinator.shutdown();
}
