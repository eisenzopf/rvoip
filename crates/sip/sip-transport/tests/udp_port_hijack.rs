//! A SIP UDP transport bound to an explicit port must own that port.
//!
//! On Linux, two UDP sockets that both set `SO_REUSEADDR` may bind the same
//! address and port, and the kernel then delivers every unicast datagram for
//! that port to the newer socket. Linux does not compare user ids for
//! `SO_REUSEADDR`, so if the transport set the flag, any local process could
//! bind the transport's port with it and silently receive the transport's SIP
//! traffic. Without the flag on the transport the kernel refuses that bind.
//!
//! On macOS and the BSDs a duplicate unicast bind needs `SO_REUSEPORT` on both
//! sockets, so the intruder is refused there whether or not the transport sets
//! `SO_REUSEADDR`: the test passes but shows nothing the Linux run does not.
//! On Windows `SO_REUSEADDR` lets a second socket bind a held port whatever
//! the holder set, unless the holder set `SO_EXCLUSIVEADDRUSE` before its
//! bind, which the transport does there. Windows reports the refused bind as
//! `WSAEADDRINUSE` or `WSAEACCES`. rvoip's CI runs on Linux only, so the
//! Windows build of this test is compile-checked, not run.
#![cfg(any(unix, windows))]

use std::io;
use std::net::{SocketAddr, UdpSocket as StdUdpSocket};
use std::time::Duration;

use rvoip_sip_transport::transport::udp::UdpTransport;
use rvoip_sip_transport::{Transport, TransportEvent};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::sync::mpsc;

const OPTIONS: &[u8] = b"OPTIONS sip:hijack@127.0.0.1 SIP/2.0\r\n\
Via: SIP/2.0/UDP 127.0.0.1:5999;branch=z9hG4bK-udp-port-hijack\r\n\
From: <sip:probe@127.0.0.1>;tag=probe\r\n\
To: <sip:hijack@127.0.0.1>\r\n\
Call-ID: udp-port-hijack@127.0.0.1\r\n\
CSeq: 1 OPTIONS\r\n\
Max-Forwards: 70\r\n\
Content-Length: 0\r\n\r\n";

/// Bind a running transport to an explicit (non-zero) loopback port.
async fn transport_on_explicit_port() -> (UdpTransport, mpsc::Receiver<TransportEvent>, SocketAddr)
{
    for _ in 0..20 {
        let port = StdUdpSocket::bind("127.0.0.1:0")
            .and_then(|probe| probe.local_addr())
            .expect("probe for a free port")
            .port();
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        // Another process can take the probed port in between; try again.
        if let Ok((transport, events)) = UdpTransport::bind(addr, None).await {
            return (transport, events, addr);
        }
    }
    panic!("could not bind a SIP UDP transport to an explicit loopback port");
}

/// Unix refuses the intruder with `EADDRINUSE`; Windows with `WSAEADDRINUSE`
/// or `WSAEACCES`.
fn refused(err: &io::Error) -> bool {
    match err.kind() {
        io::ErrorKind::AddrInUse => true,
        io::ErrorKind::PermissionDenied => cfg!(windows),
        _ => false,
    }
}

#[tokio::test]
async fn reuse_flagged_socket_cannot_take_an_explicit_sip_udp_port() {
    let (transport, mut events, addr) = transport_on_explicit_port().await;
    assert_eq!(transport.local_addr().unwrap(), addr);
    let sender = StdUdpSocket::bind("127.0.0.1:0").expect("sender socket");

    let intruder = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).unwrap();
    intruder.set_reuse_address(true).unwrap();
    match intruder.bind(&addr.into()) {
        Ok(()) => {
            let intruder: StdUdpSocket = intruder.into();
            intruder
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            sender.send_to(OPTIONS, addr).expect("send probe");
            let mut buf = [0u8; 2048];
            let outcome = match intruder.recv_from(&mut buf) {
                Ok((len, _)) => {
                    format!("and received the {len}-byte SIP request sent to the transport")
                }
                Err(err) => format!("(the probe request did not reach it: {err})"),
            };
            panic!(
                "hijack: a second SO_REUSEADDR socket bound the transport's port {addr} {outcome}"
            );
        }
        Err(err) => assert!(
            refused(&err),
            "the intruder must be refused with EADDRINUSE, got {err}"
        ),
    }

    // The transport still owns the port and receives what is sent to it.
    sender.send_to(OPTIONS, addr).expect("send probe");
    let received = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Some(TransportEvent::MessageReceived { .. }) => return true,
                Some(_) => continue,
                None => return false,
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(
        received,
        "the transport must receive the request sent to its port"
    );

    transport.close().await.unwrap();
}
