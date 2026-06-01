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
}
