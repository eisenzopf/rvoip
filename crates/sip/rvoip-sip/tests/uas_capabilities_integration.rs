//! `Config::uas_capabilities` end to end: a running endpoint checks inbound
//! requests against the configured capabilities (RFC 3261 §8.2) and
//! advertises the same ones in its OPTIONS response (RFC 3261 §11.2).
//!
//! Requests are written by hand and sent over UDP; their top Via carries the
//! test socket's port, so responses come back to it.

use std::time::Duration;

use rvoip_sip::api::unified::{Config, RelUsage, UasCapabilities, UnifiedCoordinator};
use rvoip_sip_core::types::headers::TypedHeader;
use rvoip_sip_core::types::Method;
use rvoip_sip_core::{parse_message_with_mode, Message, ParseMode, Response};
use tokio::net::UdpSocket;

const PORT: u16 = 16420;

fn request(method: &str, port: u16, local: u16, call_id: &str, extra: &str) -> String {
    format!(
        "{method} sip:uas@127.0.0.1:{port} SIP/2.0\r\n\
         Via: SIP/2.0/UDP 127.0.0.1:{local};branch=z9hG4bK-{call_id}\r\n\
         Max-Forwards: 70\r\n\
         From: <sip:test@127.0.0.1:{local}>;tag=t-{call_id}\r\n\
         To: <sip:uas@127.0.0.1:{port}>\r\n\
         Call-ID: {call_id}@uas-capabilities\r\n\
         CSeq: 1 {method}\r\n\
         Contact: <sip:test@127.0.0.1:{local}>\r\n\
         {extra}\
         Content-Length: 0\r\n\r\n"
    )
}

/// The first final response to `call_id`, which must be valid SIP.
async fn final_response(socket: &UdpSocket, call_id: &str) -> Response {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut datagram = vec![0u8; 65535];
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let (n, _) = tokio::time::timeout(remaining, socket.recv_from(&mut datagram))
            .await
            .unwrap_or_else(|_| panic!("no final response to {call_id}"))
            .expect("receive");
        let message = parse_message_with_mode(&datagram[..n], ParseMode::Strict)
            .unwrap_or_else(|error| panic!("response is not valid SIP ({error})"));
        if let Message::Response(response) = message {
            let ours = response
                .call_id()
                .is_some_and(|id| id.value().starts_with(call_id));
            if ours && response.status_code() >= 200 {
                return response;
            }
        }
    }
}

fn header_text(response: &Response, matches: fn(&TypedHeader) -> bool) -> String {
    response
        .headers
        .iter()
        .filter(|header| matches(header))
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

async fn endpoint(config: Config) -> (std::sync::Arc<UnifiedCoordinator>, UdpSocket, u16) {
    let coordinator = UnifiedCoordinator::new(config).await.expect("coordinator");
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let local = socket.local_addr().unwrap().port();
    tokio::time::sleep(Duration::from_millis(200)).await;
    (coordinator, socket, local)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn configured_capabilities_are_enforced_and_advertised() {
    let port = PORT;
    let mut config = Config::local("uas-capabilities", port);
    let mut capabilities = UasCapabilities::default();
    capabilities
        .methods
        .retain(|method| *method != Method::Message);
    capabilities
        .supported_extensions
        .push("x-application-feature".to_string());
    config.uas_capabilities = capabilities;
    let (coordinator, socket, local) = endpoint(config).await;
    let send = |text: String| {
        let socket = &socket;
        async move {
            socket
                .send_to(text.as_bytes(), ("127.0.0.1", port))
                .await
                .unwrap();
        }
    };

    // OPTIONS lists exactly the configured capabilities.
    send(request("OPTIONS", port, local, "options", "")).await;
    let options = final_response(&socket, "options").await;
    assert_eq!(options.status_code(), 200);
    let allow = header_text(&options, |h| matches!(h, TypedHeader::Allow(_)));
    assert!(
        allow.contains("INVITE") && allow.contains("PRACK"),
        "{allow}"
    );
    assert!(!allow.contains("MESSAGE"), "MESSAGE was removed: {allow}");
    let supported = header_text(&options, |h| matches!(h, TypedHeader::Supported(_)));
    assert!(supported.contains("x-application-feature"), "{supported}");
    assert!(
        supported.contains("100rel") && supported.contains("timer"),
        "{supported}"
    );
    assert!(!supported.contains("replaces"), "{supported}");
    let accept = header_text(&options, |h| matches!(h, TypedHeader::Accept(_)));
    assert!(accept.contains("application/sdp"), "{accept}");

    // A method removed from the capabilities gets 405 with the same Allow.
    send(request("MESSAGE", port, local, "message", "")).await;
    let message = final_response(&socket, "message").await;
    assert_eq!(message.status_code(), 405);
    assert_eq!(
        header_text(&message, |h| matches!(h, TypedHeader::Allow(_))),
        allow
    );

    // An added option tag is accepted in Require.
    send(request(
        "OPTIONS",
        port,
        local,
        "required-feature",
        "Require: x-application-feature\r\n",
    ))
    .await;
    assert_eq!(
        final_response(&socket, "required-feature")
            .await
            .status_code(),
        200
    );

    // `replaces` is not implemented, so requiring it gets 420.
    send(request(
        "OPTIONS",
        port,
        local,
        "replaces",
        "Require: replaces\r\n",
    ))
    .await;
    let replaces = final_response(&socket, "replaces").await;
    assert_eq!(replaces.status_code(), 420);
    let unsupported = header_text(&replaces, |h| matches!(h, TypedHeader::Unsupported(_)));
    assert!(unsupported.contains("replaces"), "{unsupported}");

    coordinator.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn disabled_100rel_is_not_supported() {
    let port = PORT + 1;
    let mut config = Config::local("uas-capabilities-no-100rel", port);
    config.use_100rel = RelUsage::NotSupported;
    let (coordinator, socket, local) = endpoint(config).await;

    socket
        .send_to(
            request("OPTIONS", port, local, "rel", "Require: 100rel\r\n").as_bytes(),
            ("127.0.0.1", port),
        )
        .await
        .unwrap();
    let response = final_response(&socket, "rel").await;
    assert_eq!(response.status_code(), 420);
    let unsupported = header_text(&response, |h| matches!(h, TypedHeader::Unsupported(_)));
    assert!(unsupported.contains("100rel"), "{unsupported}");

    socket
        .send_to(
            request("OPTIONS", port, local, "rel-options", "").as_bytes(),
            ("127.0.0.1", port),
        )
        .await
        .unwrap();
    let options = final_response(&socket, "rel-options").await;
    let supported = header_text(&options, |h| matches!(h, TypedHeader::Supported(_)));
    assert!(!supported.contains("100rel"), "{supported}");

    coordinator.shutdown();
}
