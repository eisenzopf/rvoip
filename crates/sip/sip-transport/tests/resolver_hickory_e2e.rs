//! RFC 3263 end-to-end smoke: run a real `hickory-server` authoritative
//! DNS instance bound to `127.0.0.1:0` with a fixture zone, point
//! `HickoryResolver` at it, and verify the NAPTR → SRV → A ladder
//! produces the expected ordered candidate list.

#![cfg(feature = "dns")]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use hickory_proto::rr::rdata::{NAPTR, SRV};
use hickory_proto::rr::{LowerName, Name, RData, Record};
use hickory_resolver::config::{NameServerConfig, ResolverConfig, ResolverOpts};
use hickory_server::store::in_memory::InMemoryZoneHandler;
use hickory_server::zone_handler::{AxfrPolicy, Catalog, ZoneHandler, ZoneType};
use hickory_server::Server;
use rvoip_sip_core::Uri;
use rvoip_sip_transport::resolver::{AddressFamily, HickoryResolver, Resolver, ResolverError};
use rvoip_sip_transport::transport::{TransportAuthority, TransportType};
use tokio::net::UdpSocket;

const TTL: u32 = 300;
const DUAL_V4: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 10);
const DUAL_V6: Ipv6Addr = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x10);

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

fn add_record(authority: &mut InMemoryZoneHandler, owner: &str, rdata: RData) {
    let record = Record::from_rdata(name(owner), TTL, rdata);
    authority.upsert_mut(record, 0);
}

fn build_fixture_authority() -> InMemoryZoneHandler {
    let origin = name("example.test.");
    let mut authority =
        InMemoryZoneHandler::empty(origin.clone(), ZoneType::Primary, AxfrPolicy::Deny);

    // Top-level domain NAPTRs — cover every RFC 3263 transport service token
    // supported by this release profile. Lower order values establish a
    // deterministic TLS, TCP, UDP candidate ordering.
    add_record(
        &mut authority,
        "example.test.",
        naptr_rdata(10, 50, "SIPS+D2T", "_sips._tcp.example.test."),
    );
    add_record(
        &mut authority,
        "example.test.",
        naptr_rdata(20, 50, "SIP+D2T", "_sip._tcp.example.test."),
    );
    add_record(
        &mut authority,
        "example.test.",
        naptr_rdata(30, 50, "SIP+D2U", "_sip._udp.example.test."),
    );

    // SRVs for both transports point at host.example.test.
    add_record(
        &mut authority,
        "_sips._tcp.example.test.",
        srv_rdata(1, 1, 5061, "host.example.test."),
    );
    add_record(
        &mut authority,
        "_sip._tcp.example.test.",
        srv_rdata(1, 1, 5060, "host.example.test."),
    );
    add_record(
        &mut authority,
        "_sip._udp.example.test.",
        srv_rdata(1, 1, 5060, "host.example.test."),
    );

    // A record for the SRV target — same loopback for both flavours.
    add_record(&mut authority, "host.example.test.", host_rdata());

    // A dual-stack host (A + AAAA) reached through its own SRV, used by the
    // address-family tests.
    add_record(
        &mut authority,
        "dual.example.test.",
        RData::A(hickory_proto::rr::rdata::A(DUAL_V4)),
    );
    add_record(
        &mut authority,
        "dual.example.test.",
        RData::AAAA(hickory_proto::rr::rdata::AAAA(DUAL_V6)),
    );
    add_record(
        &mut authority,
        "_sip._udp.dualsrv.example.test.",
        srv_rdata(1, 1, 5070, "dual.example.test."),
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
    catalog.upsert(
        LowerName::from(name("example.test.")),
        vec![authority as Arc<dyn ZoneHandler>],
    );

    let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind UDP");
    let local = socket.local_addr().expect("local_addr");

    let mut server = Server::new(catalog);
    server.register_socket(socket);
    let handle = tokio::spawn(async move {
        // `block_until_done` consumes the server. When the test holds
        // the JoinHandle, the server runs until the task is aborted.
        let _ = server.block_until_done().await;
    });

    // Build resolver pointed at the fixture.
    let mut name_server = NameServerConfig::udp(local.ip());
    name_server.connections[0].port = local.port();
    let config = ResolverConfig::from_parts(None, vec![], vec![name_server]);
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

    // Expected: TLS, TCP, then UDP from NAPTR orders 10, 20, and 30. All
    // should map to 127.0.0.1 with the SRV-supplied ports.
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
        candidates.iter().any(|c| c.transport == TransportType::Tcp
            && c.addr == SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 5060)),
        "missing TCP candidate at 127.0.0.1:5060 — got {:?}",
        candidates
    );
    assert!(
        candidates.iter().any(|c| c.transport == TransportType::Udp
            && c.addr == SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 5060)),
        "missing UDP candidate at 127.0.0.1:5060 — got {:?}",
        candidates
    );
    // The complete NAPTR service ordering is honoured.
    let tls_idx = candidates
        .iter()
        .position(|c| c.transport == TransportType::Tls)
        .unwrap();
    let tcp_idx = candidates
        .iter()
        .position(|c| c.transport == TransportType::Tcp)
        .unwrap();
    let udp_idx = candidates
        .iter()
        .position(|c| c.transport == TransportType::Udp)
        .unwrap();
    assert!(
        tls_idx < tcp_idx && tcp_idx < udp_idx,
        "NAPTR order should be TLS, TCP, UDP; got TLS@{} TCP@{} UDP@{}",
        tls_idx,
        tcp_idx,
        udp_idx
    );
    assert!(
        candidates.iter().all(|candidate| {
            candidate.authority == Some(TransportAuthority::Dns("example.test".to_string()))
        }),
        "NAPTR/SRV targets must retain the original SIP authority for TLS SNI: {candidates:?}"
    );

    server.abort();
}

#[tokio::test]
async fn unrestricted_resolver_returns_both_families() {
    let (_addr, resolver, server) = spin_up_fixture().await;
    let uri = Uri::from_str("sip:dual.example.test:5070").unwrap();
    let candidates = resolver.resolve(&uri).await.expect("resolve");
    let addrs: Vec<IpAddr> = candidates.iter().map(|c| c.addr.ip()).collect();
    assert!(addrs.contains(&IpAddr::V4(DUAL_V4)), "{candidates:?}");
    assert!(addrs.contains(&IpAddr::V6(DUAL_V6)), "{candidates:?}");
    server.abort();
}

#[tokio::test]
async fn ipv4_restricted_resolver_never_returns_aaaa_targets() {
    let (_addr, resolver, server) = spin_up_fixture().await;
    let resolver = resolver.with_address_family(Some(AddressFamily::Ipv4));
    for uri in [
        "sip:dual.example.test:5070",
        "sip:dualsrv.example.test;transport=udp",
    ] {
        let candidates = resolver
            .resolve(&Uri::from_str(uri).unwrap())
            .await
            .expect("resolve");
        assert_eq!(
            candidates.iter().map(|c| c.addr).collect::<Vec<_>>(),
            vec![SocketAddr::new(IpAddr::V4(DUAL_V4), 5070)],
            "{uri}"
        );
    }
    server.abort();
}

#[tokio::test]
async fn ipv6_restricted_resolver_never_returns_a_targets() {
    let (_addr, resolver, server) = spin_up_fixture().await;
    let resolver = resolver.with_address_family(Some(AddressFamily::Ipv6));
    for uri in [
        "sip:dual.example.test:5070",
        "sip:dualsrv.example.test;transport=udp",
    ] {
        let candidates = resolver
            .resolve(&Uri::from_str(uri).unwrap())
            .await
            .expect("resolve");
        assert_eq!(
            candidates.iter().map(|c| c.addr).collect::<Vec<_>>(),
            vec![SocketAddr::new(IpAddr::V6(DUAL_V6), 5070)],
            "{uri}"
        );
    }

    // `example.test`'s NAPTR/SRV ladder only reaches an A record, so an
    // IPv6-only stack has no candidate rather than an unreachable one.
    let error = resolver
        .resolve(&Uri::from_str("sip:example.test").unwrap())
        .await
        .expect_err("no AAAA anywhere in the ladder");
    assert!(matches!(error, ResolverError::NoCandidates), "{error:?}");
    server.abort();
}
