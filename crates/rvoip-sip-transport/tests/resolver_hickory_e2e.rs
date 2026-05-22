//! RFC 3263 end-to-end smoke: run a real `hickory-server` authoritative
//! DNS instance bound to `127.0.0.1:0` with a fixture zone, point
//! `HickoryResolver` at it, and verify the NAPTR → SRV → A ladder
//! produces the expected ordered candidate list.

#![cfg(feature = "dns")]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use hickory_proto::rr::rdata::{NAPTR, SRV};
use hickory_proto::rr::{LowerName, Name, RData, Record, RecordType};
use hickory_resolver::config::ResolverOpts;
use hickory_resolver::config::{NameServerConfig, NameServerConfigGroup, Protocol, ResolverConfig};
use hickory_server::authority::{Catalog, ZoneType};
use hickory_server::store::in_memory::InMemoryAuthority;
use hickory_server::ServerFuture;
use rvoip_sip_core::Uri;
use rvoip_sip_transport::resolver::{HickoryResolver, Resolver};
use rvoip_sip_transport::transport::TransportType;
use tokio::net::UdpSocket;

const TTL: u32 = 300;

fn name(s: &str) -> Name {
    Name::from_str(s).expect("valid DNS name")
}

fn host_rdata() -> RData {
    RData::A(hickory_proto::rr::rdata::A(Ipv4Addr::new(127, 0, 0, 1)))
}

fn srv_rdata(priority: u16, weight: u16, port: u16, target: &str) -> RData {
    RData::SRV(SRV::new(priority, weight, port, name(target)))
}

fn naptr_rdata(order: u16, preference: u16, service: &str, replacement: &str) -> RData {
    RData::NAPTR(NAPTR::new(
        order,
        preference,
        // flags: "s" — replacement is an SRV target (RFC 3263 §4.1).
        "s".as_bytes().to_vec().into_boxed_slice(),
        // services token, e.g. "SIPS+D2T".
        service.as_bytes().to_vec().into_boxed_slice(),
        // regexp: empty (we use replacement, not regexp, per RFC 3263).
        Box::default(),
        // replacement target.
        name(replacement),
    ))
}

fn add_record(authority: &mut InMemoryAuthority, owner: &str, rtype: RecordType, rdata: RData) {
    let mut record = Record::with(name(owner), rtype, TTL);
    record.set_data(Some(rdata));
    authority.upsert_mut(record, 0);
}

fn build_fixture_authority() -> InMemoryAuthority {
    let origin = name("example.test.");
    let mut authority = InMemoryAuthority::empty(
        origin.clone(),
        ZoneType::Primary,
        /* allow_axfr */ false,
    );

    // Top-level domain NAPTRs — SIPS+D2T first (lower order), then SIP+D2U.
    add_record(
        &mut authority,
        "example.test.",
        RecordType::NAPTR,
        naptr_rdata(10, 50, "SIPS+D2T", "_sips._tcp.example.test."),
    );
    add_record(
        &mut authority,
        "example.test.",
        RecordType::NAPTR,
        naptr_rdata(20, 50, "SIP+D2U", "_sip._udp.example.test."),
    );

    // SRVs for both transports point at host.example.test.
    add_record(
        &mut authority,
        "_sips._tcp.example.test.",
        RecordType::SRV,
        srv_rdata(1, 1, 5061, "host.example.test."),
    );
    add_record(
        &mut authority,
        "_sip._udp.example.test.",
        RecordType::SRV,
        srv_rdata(1, 1, 5060, "host.example.test."),
    );

    // A record for the SRV target — same loopback for both flavours.
    add_record(
        &mut authority,
        "host.example.test.",
        RecordType::A,
        host_rdata(),
    );

    authority
}

/// Bind a real `hickory-server` to a free UDP port on the loopback and
/// return the resolver pointed at it. Caller keeps the returned join
/// handle alive for the duration of the test; dropping it shuts the
/// server down.
async fn spin_up_fixture() -> (SocketAddr, HickoryResolver, tokio::task::JoinHandle<()>) {
    let authority = Arc::new(build_fixture_authority());
    let mut catalog = Catalog::new();
    catalog.upsert(LowerName::from(name("example.test.")), Box::new(authority));

    let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind UDP");
    let local = socket.local_addr().expect("local_addr");

    let mut server = ServerFuture::new(catalog);
    server.register_socket(socket);
    let handle = tokio::spawn(async move {
        // `block_until_done` consumes the server. When the test holds
        // the JoinHandle, the server runs until the task is aborted.
        let _ = server.block_until_done().await;
    });

    // Build resolver pointed at the fixture.
    let mut ns_group = NameServerConfigGroup::new();
    ns_group.push(NameServerConfig::new(local, Protocol::Udp));
    let config = ResolverConfig::from_parts(None, vec![], ns_group);
    let mut opts = ResolverOpts::default();
    opts.timeout = Duration::from_secs(2);
    let resolver = HickoryResolver::with_resolver(config, opts);

    // Give the server a moment to start listening.
    tokio::time::sleep(Duration::from_millis(20)).await;
    (local, resolver, handle)
}

#[tokio::test]
async fn hickory_client_resolves_naptr_then_srv_then_a() {
    let (_addr, resolver, server) = spin_up_fixture().await;
    let uri = Uri::from_str("sip:example.test").unwrap();
    let candidates = resolver
        .resolve(&uri)
        .await
        .expect("resolve must succeed against fixture");

    // Expected: TLS (from the order=10 NAPTR) first, then UDP
    // (order=20). Both should map to 127.0.0.1 with the SRV-supplied
    // ports.
    assert!(
        !candidates.is_empty(),
        "resolver returned no candidates against fixture"
    );
    assert!(
        candidates.iter().any(|c| c.transport == TransportType::Tls
            && c.addr == SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 5061)),
        "missing TLS candidate at 127.0.0.1:5061 — got {:?}",
        candidates
    );
    assert!(
        candidates.iter().any(|c| c.transport == TransportType::Udp
            && c.addr == SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 5060)),
        "missing UDP candidate at 127.0.0.1:5060 — got {:?}",
        candidates
    );
    // TLS appears before UDP (RFC 3263 NAPTR order honoured).
    let tls_idx = candidates
        .iter()
        .position(|c| c.transport == TransportType::Tls)
        .unwrap();
    let udp_idx = candidates
        .iter()
        .position(|c| c.transport == TransportType::Udp)
        .unwrap();
    assert!(
        tls_idx < udp_idx,
        "TLS should precede UDP (lower NAPTR order); got TLS@{} UDP@{}",
        tls_idx,
        udp_idx
    );

    server.abort();
}
