use std::net::SocketAddr;

use rvoip_sip_core::prelude::{Host, Request, Response};
use rvoip_sip_transport::{transport::TransportType, TransportRoute};

/// Top Via identity emitted by one client transaction.
///
/// RFC 3261 section 18.1.2 requires a client transport to reject responses
/// whose top Via sent-by is not one it is configured to place in requests.
/// Retaining the exact emitted value also supports builders that advertise a
/// sent-by address different from the transport's default local socket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClientResponseViaIdentity {
    transport: TransportType,
    sent_by_host: Host,
    sent_by_port: Option<u16>,
}

impl ClientResponseViaIdentity {
    pub(crate) fn from_request(request: &Request) -> Option<Self> {
        let via = request.first_via()?;
        let top_via = via.0.first()?;
        Some(Self {
            transport: parse_transport(&top_via.sent_protocol.transport)?,
            sent_by_host: top_via.sent_by_host.clone(),
            sent_by_port: top_via.sent_by_port,
        })
    }

    fn matches_response(&self, response: &Response) -> bool {
        let Some(via) = response.first_via() else {
            return false;
        };
        via.0.first().is_some_and(|top_via| {
            parse_transport(&top_via.sent_protocol.transport) == Some(self.transport)
                && hosts_match(&top_via.sent_by_host, &self.sent_by_host)
                && top_via.sent_by_port == self.sent_by_port
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        Self {
            transport: TransportType::Udp,
            sent_by_host: Host::Address(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
            sent_by_port: Some(5060),
        }
    }
}

fn parse_transport(value: &str) -> Option<TransportType> {
    if value.eq_ignore_ascii_case("UDP") {
        Some(TransportType::Udp)
    } else if value.eq_ignore_ascii_case("TCP") {
        Some(TransportType::Tcp)
    } else if value.eq_ignore_ascii_case("TLS") {
        Some(TransportType::Tls)
    } else if value.eq_ignore_ascii_case("WS") {
        Some(TransportType::Ws)
    } else if value.eq_ignore_ascii_case("WSS") {
        Some(TransportType::Wss)
    } else {
        None
    }
}

fn hosts_match(received: &Host, expected: &Host) -> bool {
    match (received, expected) {
        (Host::Domain(received), Host::Domain(expected)) => received.eq_ignore_ascii_case(expected),
        (Host::Address(received), Host::Address(expected)) => received == expected,
        _ => false,
    }
}

pub(super) fn client_response_route_matches(
    expected: &TransportRoute,
    expected_via: &ClientResponseViaIdentity,
    response: &Response,
    source: SocketAddr,
    transport_type: TransportType,
    ingress_has_flow: bool,
) -> bool {
    if expected.transport_type != Some(transport_type) || !expected_via.matches_response(response) {
        return false;
    }

    match transport_type {
        TransportType::Udp => expected.destination == source && !ingress_has_flow,
        TransportType::Tcp | TransportType::Tls | TransportType::Ws | TransportType::Wss => {
            // A reliable response can arrive on a newly opened connection.
            // Remote address and process-local flow ID are not RFC 3261
            // client-transaction matching keys.
            ingress_has_flow
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::{
        builder::{SimpleRequestBuilder, SimpleResponseBuilder},
        prelude::Method,
        types::status::StatusCode,
    };

    fn exchange(via: &str, via_transport: &str) -> (ClientResponseViaIdentity, Response) {
        let request = SimpleRequestBuilder::new(Method::Options, "sip:peer.example.test")
            .expect("test URI must be valid")
            .from("Alice", "sip:alice@example.test", Some("alice-tag"))
            .to("Peer", "sip:peer.example.test", None)
            .call_id("response-route-test")
            .cseq(1)
            .via(via, via_transport, Some("z9hG4bK.response-route"))
            .max_forwards(70)
            .build();
        let identity = ClientResponseViaIdentity::from_request(&request)
            .expect("test request must contain a Via identity");
        let response =
            SimpleResponseBuilder::response_from_request(&request, StatusCode::Ok, Some("OK"))
                .build();
        (identity, response)
    }

    #[test]
    fn reliable_response_accepts_an_alternate_ingress_flow() {
        let (identity, response) = exchange("192.0.2.10:5070", "TCP");
        let route = TransportRoute::new("192.0.2.20:5060".parse().expect("valid route"))
            .with_transport_type(TransportType::Tcp);

        assert!(client_response_route_matches(
            &route,
            &identity,
            &response,
            "198.51.100.40:5090".parse().expect("valid source"),
            TransportType::Tcp,
            true,
        ));
        assert!(!client_response_route_matches(
            &route,
            &identity,
            &response,
            route.destination,
            TransportType::Tcp,
            false,
        ));
    }

    #[test]
    fn sent_by_domain_and_transport_matching_is_case_insensitive() {
        let (identity, _) = exchange("CLIENT.Example.Test:5070", "tcp");
        let (_, response) = exchange("client.example.test:5070", "TCP");
        let route = TransportRoute::new("192.0.2.20:5060".parse().expect("valid route"))
            .with_transport_type(TransportType::Tcp);

        assert!(client_response_route_matches(
            &route,
            &identity,
            &response,
            route.destination,
            TransportType::Tcp,
            true,
        ));
    }

    #[test]
    fn response_rejects_a_different_sent_by_or_transport() {
        let (identity, response) = exchange("192.0.2.10:5070", "TCP");
        let (_, wrong_via_response) = exchange("192.0.2.11:5070", "TCP");
        let route = TransportRoute::new("192.0.2.20:5060".parse().expect("valid route"))
            .with_transport_type(TransportType::Tcp);

        assert!(!client_response_route_matches(
            &route,
            &identity,
            &wrong_via_response,
            route.destination,
            TransportType::Tcp,
            true,
        ));
        assert!(!client_response_route_matches(
            &route,
            &identity,
            &response,
            route.destination,
            TransportType::Tls,
            true,
        ));
    }

    #[test]
    fn udp_keeps_exact_source_matching_without_a_flow() {
        let (identity, response) = exchange("192.0.2.10:5060", "UDP");
        let source = "192.0.2.20:5060".parse().expect("valid source");
        let route = TransportRoute::new(source).with_transport_type(TransportType::Udp);

        assert!(client_response_route_matches(
            &route,
            &identity,
            &response,
            source,
            TransportType::Udp,
            false,
        ));
        assert!(!client_response_route_matches(
            &route,
            &identity,
            &response,
            "192.0.2.21:5060".parse().expect("valid source"),
            TransportType::Udp,
            false,
        ));
        assert!(!client_response_route_matches(
            &route,
            &identity,
            &response,
            source,
            TransportType::Udp,
            true,
        ));
    }
}
