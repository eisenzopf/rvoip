use std::io;
use std::net::{SocketAddr, UdpSocket as StdUdpSocket};

use socket2::{Domain, Protocol, Socket, Type};

/// Optional UDP socket sizing applied before bind.
///
/// Defaults preserve platform behavior. Server deployments can set these when
/// expected call bursts exceed the OS default UDP queue depth.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UdpSocketOptions {
    /// Requested `SO_RCVBUF` size in bytes.
    pub recv_buffer_size: Option<usize>,
    /// Requested `SO_SNDBUF` size in bytes.
    pub send_buffer_size: Option<usize>,
}

impl UdpSocketOptions {
    /// Construct socket options from optional receive/send buffer sizes.
    pub const fn new(recv_buffer_size: Option<usize>, send_buffer_size: Option<usize>) -> Self {
        Self {
            recv_buffer_size,
            send_buffer_size,
        }
    }
}

pub(crate) fn bind_std_udp_socket(
    addr: SocketAddr,
    options: UdpSocketOptions,
) -> io::Result<StdUdpSocket> {
    let domain = if addr.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };
    let socket = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))?;

    if let Some(size) = options.recv_buffer_size {
        socket.set_recv_buffer_size(size)?;
    }
    if let Some(size) = options.send_buffer_size {
        socket.set_send_buffer_size(size)?;
    }

    // No SO_REUSEADDR, on any port. UDP has no TIME_WAIT: once the previous
    // socket on a port is closed (dropped, or its process exits or is killed)
    // the kernel frees the port and a plain bind of it succeeds at once.
    //
    // The flag only matters while another socket still holds the port. On
    // Linux two UDP sockets that both set it may bind the same address and
    // port, from any process and any user, and the newer one then receives
    // every datagram sent there: a transport that set it could have its SIP
    // traffic silently taken by any other reuse-flagged socket, and a port-0
    // bind could be handed a port a reuse-flagged socket already held.
    //
    // macOS and the BSDs need SO_REUSEPORT on both sockets for a duplicate
    // unicast bind, so there the flag never let a second bind of a held port
    // succeed. Windows differs again: a later socket that sets SO_REUSEADDR
    // can bind a held port whatever the holder set, unless the holder sets
    // SO_EXCLUSIVEADDRUSE.
    socket.bind(&addr.into())?;
    socket.set_nonblocking(true)?;
    Ok(socket.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_std_udp_socket_accepts_explicit_buffers() {
        let socket = bind_std_udp_socket(
            "127.0.0.1:0".parse().unwrap(),
            UdpSocketOptions::new(Some(4096), Some(4096)),
        )
        .expect("bind with socket options");

        assert!(socket.local_addr().unwrap().port() > 0);
    }

    #[test]
    fn rebinds_same_port_after_drop() {
        // Discover a free port, release it, then immediately rebind the same
        // port. UDP has no TIME_WAIT, so the rebind needs no SO_REUSEADDR.
        let probe =
            bind_std_udp_socket("127.0.0.1:0".parse().unwrap(), UdpSocketOptions::default())
                .expect("probe bind");
        let addr = probe.local_addr().unwrap();
        drop(probe);

        let rebind =
            bind_std_udp_socket(addr, UdpSocketOptions::default()).expect("rebind same port");
        assert_eq!(rebind.local_addr().unwrap().port(), addr.port());
    }

    /// A second socket that sets `SO_REUSEADDR` must not be able to bind the
    /// transport's port. On Linux it could if the transport also set the
    /// flag, and the newer socket then receives every datagram sent to that
    /// port, silently taking this transport's SIP traffic.
    #[cfg(unix)]
    fn assert_port_not_shareable(addr: SocketAddr) {
        let intruder = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).unwrap();
        intruder.set_reuse_address(true).unwrap();
        let err = intruder
            .bind(&addr.into())
            .expect_err("a second socket must not share a SIP UDP port");
        assert_eq!(err.kind(), io::ErrorKind::AddrInUse);
    }

    #[cfg(unix)]
    #[test]
    fn ephemeral_bind_does_not_share_its_port() {
        let ephemeral =
            bind_std_udp_socket("127.0.0.1:0".parse().unwrap(), UdpSocketOptions::default())
                .expect("ephemeral bind");
        assert_port_not_shareable(ephemeral.local_addr().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn explicit_bind_does_not_share_its_port() {
        let probe =
            bind_std_udp_socket("127.0.0.1:0".parse().unwrap(), UdpSocketOptions::default())
                .expect("probe bind");
        let addr = probe.local_addr().unwrap();
        drop(probe);

        let explicit =
            bind_std_udp_socket(addr, UdpSocketOptions::default()).expect("explicit-port bind");
        assert_port_not_shareable(explicit.local_addr().unwrap());
    }
}
