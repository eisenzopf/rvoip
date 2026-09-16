use rvoip_sip_core::{Request, Uri};
use rvoip_sip_transport::resolver::ResolvedTarget;
use rvoip_sip_transport::transport::TransportType;
use rvoip_sip_transport::TransportRoute;

use crate::transaction::error::{Error, Result};
use crate::transaction::transport::multiplexed::{top_route_uri, transport_route_for_request};

use super::utils::socket_addr_from_uri;

/// Next hop selected for a dialog-forming ACK.
#[derive(Debug)]
pub(super) enum AckNextHop {
    Route(TransportRoute),
    /// The top Route names a host that needs RFC 3263 resolution.
    Resolve(Uri),
}

/// Derive the transport route for a dialog-forming ACK.
///
/// A response route set replaces the INVITE's next hop. In that case every
/// route identity field comes from the ACK's top Route URI; carrying the old
/// authority or transport into a new connection can select the wrong child
/// transport or authenticate the wrong peer. An exact stream flow remains
/// reusable only while the complete connection identity remains compatible.
pub(super) fn route_for_2xx_ack(ack: &Request, original: &TransportRoute) -> Result<AckNextHop> {
    let top_route = top_route_uri(ack);
    let next_hop = top_route.as_ref().unwrap_or_else(|| ack.uri());
    let destination = match socket_addr_from_uri(next_hop) {
        Some(destination) => destination,
        None if top_route.is_none() => original.destination,
        None => return Ok(AckNextHop::Resolve(next_hop.clone())),
    };

    if top_route.is_none() && destination == original.destination {
        return Ok(AckNextHop::Route(original.clone()));
    }

    let selected = transport_route_for_request(ack, destination).map_err(Error::from)?;
    Ok(AckNextHop::Route(reuse_compatible_flow(original, selected)))
}

/// Build the ACK route for one RFC 3263 candidate of its top Route.
pub(super) fn route_for_resolved_ack_target(
    ack: &Request,
    original: &TransportRoute,
    target: &ResolvedTarget,
) -> Result<TransportRoute> {
    let mut selected = transport_route_for_request(ack, target.addr).map_err(Error::from)?;
    selected.transport_type = Some(target.transport);
    if let Some(authority) = &target.authority {
        selected.authority = Some(authority.clone());
    }
    Ok(reuse_compatible_flow(original, selected))
}

fn reuse_compatible_flow(
    original: &TransportRoute,
    mut selected: TransportRoute,
) -> TransportRoute {
    if flow_is_compatible(original, &selected) {
        selected.flow_id = original.flow_id;
    }
    selected
}

fn flow_is_compatible(original: &TransportRoute, selected: &TransportRoute) -> bool {
    if original.destination != selected.destination
        || original.transport_type != selected.transport_type
    {
        return false;
    }

    match selected.transport_type {
        Some(TransportType::Tls | TransportType::Wss) => original.authority == selected.authority,
        Some(TransportType::Tcp | TransportType::Ws | TransportType::Udp) | None => true,
    }
}
