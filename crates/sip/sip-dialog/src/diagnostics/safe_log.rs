//! Metadata-only values for production SIP dialog diagnostics.
//!
//! SIP identifiers, URIs, extension methods, header values, bodies, reason
//! phrases, and parser errors are all controlled by either a caller or peer.
//! They must not cross a tracing boundary.  These helpers deliberately retain
//! only fixed classifications that are useful for operations.

use rvoip_sip_core::Method;

/// Return a fixed, non-reflecting method class.
pub(crate) const fn method_class(method: &Method) -> &'static str {
    match method {
        Method::Invite => "INVITE",
        Method::Ack => "ACK",
        Method::Bye => "BYE",
        Method::Cancel => "CANCEL",
        Method::Register => "REGISTER",
        Method::Options => "OPTIONS",
        Method::Subscribe => "SUBSCRIBE",
        Method::Notify => "NOTIFY",
        Method::Publish => "PUBLISH",
        Method::Info => "INFO",
        Method::Refer => "REFER",
        Method::Message => "MESSAGE",
        Method::Prack => "PRACK",
        Method::Update => "UPDATE",
        Method::Extension(_) => "extension",
    }
}

/// Collapse untrusted errors into a fixed operation result.
pub(crate) const fn error_class<T>(_: &T) -> &'static str {
    "operation_failed"
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "outer-log-secret-canary\r\nX-Leak: yes";

    #[test]
    fn extension_methods_are_never_reflected() {
        let method = Method::Extension(SECRET.to_owned());
        let rendered = method_class(&method);
        assert_eq!(rendered, "extension");
        assert!(!rendered.contains(SECRET));
    }

    #[test]
    fn errors_are_never_reflected() {
        let rendered = error_class(&SECRET);
        assert_eq!(rendered, "operation_failed");
        assert!(!rendered.contains(SECRET));
    }

    #[test]
    fn sensitive_outer_log_shapes_cannot_return() {
        let sources = [
            include_str!("../api/client.rs"),
            include_str!("../api/common.rs"),
            include_str!("../api/unified.rs"),
            include_str!("../api/server/response_builder.rs"),
            include_str!("../api/server/sip_methods.rs"),
            include_str!("../dialog/dialog_utils.rs"),
            include_str!("../events/event_hub.rs"),
            include_str!("../manager/core.rs"),
            include_str!("../manager/dialog_operations.rs"),
            include_str!("../manager/response_lifecycle.rs"),
            include_str!("../manager/unified.rs"),
            include_str!("../protocol/invite_handler.rs"),
            include_str!("../protocol/register_handler.rs"),
            include_str!("../protocol/response_handler.rs"),
        ];
        let source = sources.join("\n");
        for forbidden in [
            "Publishing dialog event: {:?}",
            "Publishing session coordination event: {:?}",
            "Publishing cross-crate event directly: {:?}",
            "emit_session_coordination_event called with event: {:?}",
            "Received unassociated transaction event: {:?}",
            "Default resolver returned error for {}: {}",
            "Making outgoing call from {} to {}",
            "Creating outgoing dialog from {} to {}",
            "Sending response for transaction {}",
            "Ringing response with Contact: {}",
            "REGISTER response: {} {}",
            "transaction={}, status={} {}",
            "ACK contains SDP body: {}",
            "flow_key = ?",
            "error = %e",
        ] {
            assert!(
                !source.contains(forbidden),
                "sensitive log shape returned: {forbidden}"
            );
        }
    }
}
