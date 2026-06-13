use bytes::Bytes;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tracing::trace;
// `error!` is only reached from the `#[cfg(test)] pub fn default()`
// fallback path below; gate the import the same way to avoid an
// unused-import warning in the lib build.
use super::socket::{bind_std_udp_socket, UdpSocketOptions};
use crate::error::{Error, Result};
#[cfg(test)]
use tracing::error;

// Reserved for the upcoming oversized-datagram drop path; kept so the
// constant has one canonical home when that lands.
#[allow(dead_code)]
const MAX_UDP_PACKET_SIZE: usize = 65_507;
// Buffer size for receiving packets
const UDP_BUFFER_SIZE: usize = 8192;

/// UDP listener for receiving SIP messages
pub struct UdpListener {
    socket: Arc<UdpSocket>,
    /// Bound local address, captured once at `bind` time. Reading it
    /// from the kernel via `socket.local_addr()` was previously a
    /// syscall on every `receive()` even though the value is fixed
    /// for the listener's lifetime.
    local_addr: SocketAddr,
}

impl UdpListener {
    /// Binds the UDP listener to the specified address
    pub async fn bind(addr: SocketAddr) -> Result<Self> {
        Self::bind_with_socket_options(addr, UdpSocketOptions::default()).await
    }

    /// Binds the UDP listener with explicit socket options.
    pub async fn bind_with_socket_options(
        addr: SocketAddr,
        socket_options: UdpSocketOptions,
    ) -> Result<Self> {
        let std_socket =
            bind_std_udp_socket(addr, socket_options).map_err(|e| Error::BindFailed(addr, e))?;
        let socket = UdpSocket::from_std(std_socket).map_err(|e| Error::BindFailed(addr, e))?;

        let local_addr = socket.local_addr().map_err(Error::LocalAddrFailed)?;

        Ok(Self {
            socket: Arc::new(socket),
            local_addr,
        })
    }

    /// Returns a reference to the underlying socket
    pub fn socket(&self) -> &UdpSocket {
        &self.socket
    }

    /// Returns a cloned Arc to the underlying socket
    pub fn clone_socket(&self) -> Arc<UdpSocket> {
        self.socket.clone()
    }

    /// Returns the local address this listener is bound to
    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.local_addr)
    }

    /// Receives a packet from the UDP socket
    pub async fn receive(&self) -> Result<(Bytes, SocketAddr, SocketAddr)> {
        // Stack-allocated receive buffer. Previously we allocated a
        // fresh 8 KiB `BytesMut` per packet and zero-filled it with
        // `resize(_, 0)` — ~480 MB/s of heap churn at 60K req/s. The
        // stack buffer is reused for every call (tokio stores it in
        // the receive task's stack frame across awaits) and the
        // `Bytes::copy_from_slice` below only allocates the exact
        // packet length, which is ~500 B for typical SIP messages.
        let mut buf = [0u8; UDP_BUFFER_SIZE];

        let (len, src) = self
            .socket
            .recv_from(&mut buf)
            .await
            .map_err(Error::ReceiveFailed)?;

        let packet = Bytes::copy_from_slice(&buf[..len]);
        trace!("Received {} bytes from {}", len, src);

        Ok((packet, src, self.local_addr))
    }

    /// Attempts to receive one packet without waiting.
    ///
    /// Returns `Ok(None)` when the socket has no datagram ready. This is used
    /// by the UDP transport receive loop to drain short bursts after one
    /// awaited receive has already established readiness.
    pub fn try_receive(&self) -> Result<Option<(Bytes, SocketAddr, SocketAddr)>> {
        let mut buf = [0u8; UDP_BUFFER_SIZE];

        match self.socket.try_recv_from(&mut buf) {
            Ok((len, src)) => {
                let packet = Bytes::copy_from_slice(&buf[..len]);
                trace!("Received {} bytes from {}", len, src);
                Ok(Some((packet, src, self.local_addr)))
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(Error::ReceiveFailed(error)),
        }
    }

    /// Creates a default dummy listener (used for testing)
    #[cfg(test)]
    pub fn default() -> Self {
        let socket = match std::net::UdpSocket::bind("127.0.0.1:0") {
            Ok(std_socket) => {
                if let Err(e) = std_socket.set_nonblocking(true) {
                    error!("Failed to set socket to non-blocking mode: {}", e);
                }

                match UdpSocket::from_std(std_socket) {
                    Ok(socket) => socket,
                    Err(e) => {
                        error!("Failed to create tokio socket: {}", e);
                        // Create a dummy socket (this will likely fail in real use)
                        let std_socket = std::net::UdpSocket::bind("127.0.0.1:0")
                            .expect("Failed to create dummy socket");
                        UdpSocket::from_std(std_socket).expect("Failed to create tokio socket")
                    }
                }
            }
            Err(e) => {
                error!("Failed to bind socket: {}", e);
                // Create a dummy socket (this will likely fail in real use)
                let std_socket = std::net::UdpSocket::bind("127.0.0.1:0")
                    .expect("Failed to create dummy socket");
                UdpSocket::from_std(std_socket).expect("Failed to create tokio socket")
            }
        };

        let local_addr = socket
            .local_addr()
            .unwrap_or_else(|_| "127.0.0.1:0".parse().unwrap());
        Self {
            socket: Arc::new(socket),
            local_addr,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_udp_listener_bind() {
        let listener = UdpListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        assert!(addr.port() > 0);
    }

    #[tokio::test]
    async fn test_udp_listener_receive() {
        // Bind listener to a random port
        let listener = UdpListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();

        // Create a separate socket to send data to the listener
        let sender_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();

        // Send a test message
        let test_data = b"TEST SIP MESSAGE";
        sender_socket.send_to(test_data, addr).await.unwrap();

        // Receive the message
        let (packet, src, dest) = listener.receive().await.unwrap();

        assert_eq!(&packet[..], test_data);
        assert_eq!(dest, addr);
        assert_eq!(src.ip(), sender_socket.local_addr().unwrap().ip());
    }

    #[tokio::test]
    async fn test_udp_listener_try_receive() {
        let listener = UdpListener::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let sender_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();

        sender_socket.send_to(b"TRY RECEIVE", addr).await.unwrap();

        let mut received = None;
        for _ in 0..10 {
            if let Some(packet) = listener.try_receive().unwrap() {
                received = Some(packet);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let (packet, src, dest) = received.expect("try_receive should receive sent datagram");
        assert_eq!(&packet[..], b"TRY RECEIVE");
        assert_eq!(dest, addr);
        assert_eq!(src.ip(), sender_socket.local_addr().unwrap().ip());
        assert!(listener.try_receive().unwrap().is_none());
    }
}
