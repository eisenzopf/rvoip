//! RFC 3261 §18.1.1 — outbound MTU/message-size policy helpers.
//!
//! When the dialog transport multiplexer auto-fails an oversized
//! request from UDP to TCP, the top `Via` header's `sent-protocol`
//! transport field must be updated to reflect the actual transport on
//! the wire (so the peer routes the response back the same way).
//! [`set_top_via_protocol`] performs that single-field mutation.
//!
//! All other Via state — branch, sent-by host/port, additional
//! params — is preserved.

use rvoip_sip_core::types::TypedHeader;
use rvoip_sip_core::Request;

/// Overwrite the top `Via` header's `sent-protocol` transport field
/// (e.g. `UDP` → `TCP`).
///
/// Returns `true` when the change was applied (the request had a top
/// Via), `false` when no Via was found.
///
/// The branch parameter and sent-by host/port are untouched, so the
/// transaction key derived from the branch is preserved.
pub fn set_top_via_protocol(request: &mut Request, new_protocol: &str) -> bool {
    for header in request.headers.iter_mut() {
        if let TypedHeader::Via(via) = header {
            if let Some(entry) = via.0.first_mut() {
                entry.sent_protocol.transport = new_protocol.to_string();
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    use rvoip_sip_core::types::param::Param;
    use rvoip_sip_core::types::via::Via;
    use rvoip_sip_core::Method;

    fn invite_with_udp_via() -> Request {
        SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
            .unwrap()
            .from("Alice", "sip:alice@uac.example.com", Some("alicetag"))
            .to("Bob", "sip:bob@example.com", None)
            .call_id("mtu-test")
            .cseq(1)
            .header(TypedHeader::Via(
                Via::new(
                    "SIP", "2.0", "UDP",
                    "10.0.0.5", Some(5060),
                    vec![Param::branch("z9hG4bKmtu-orig")],
                )
                .unwrap(),
            ))
            .build()
    }

    #[test]
    fn flips_udp_to_tcp_and_preserves_branch() {
        let mut request = invite_with_udp_via();
        assert!(set_top_via_protocol(&mut request, "TCP"));

        let top = request.first_via().expect("via present");
        let entry = top.headers().first().expect("Via has at least one entry");
        assert_eq!(entry.sent_protocol.transport, "TCP");
        // Branch must survive the flip — the transaction key depends on it.
        assert_eq!(top.branch(), Some("z9hG4bKmtu-orig"));
        // Sent-by host/port must survive.
        assert_eq!(entry.sent_by_port, Some(5060));
    }

    #[test]
    fn returns_false_when_no_via_present() {
        let mut request = SimpleRequestBuilder::new(Method::Options, "sip:bob@example.com")
            .unwrap()
            .from("Alice", "sip:alice@uac.example.com", Some("alicetag"))
            .to("Bob", "sip:bob@example.com", None)
            .call_id("mtu-no-via")
            .cseq(1)
            .build();
        assert!(!set_top_via_protocol(&mut request, "TCP"));
    }

    #[test]
    fn only_top_via_is_modified() {
        let mut request = invite_with_udp_via();
        // Append a second Via (older hop).
        request.headers.push(TypedHeader::Via(
            Via::new(
                "SIP", "2.0", "UDP",
                "10.0.0.99", Some(5060),
                vec![Param::branch("z9hG4bKolder")],
            )
            .unwrap(),
        ));

        assert!(set_top_via_protocol(&mut request, "TCP"));

        // Count UDP vs TCP across all Via entries — we want exactly one TCP
        // (the top entry) and the older hop to remain UDP.
        let wire = request.to_string();
        let tcp_hits = wire.matches("SIP/2.0/TCP").count();
        let udp_hits = wire.matches("SIP/2.0/UDP").count();
        assert_eq!(tcp_hits, 1, "only top Via should flip; wire: {}", wire);
        assert_eq!(udp_hits, 1, "older Via must stay UDP; wire: {}", wire);
    }
}
