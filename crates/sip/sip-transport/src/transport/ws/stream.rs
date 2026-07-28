//! Unified stream wrapper for SIP WebSocket connections.
//!
//! `tokio_tungstenite::MaybeTlsStream::Rustls(_)` wraps only a
//! *client-side* TLS stream (`tokio_rustls::client::TlsStream<S>`). For
//! the WSS server-side accept path we need a wrapper that can hold a
//! `tokio_rustls::server::TlsStream<TcpStream>` as well, so this enum
//! covers all three shapes:
//!
//!   - `Plain` — `ws://` (and the client side of the original
//!     `MaybeTlsStream::Plain`)
//!   - `ClientTls` — outbound `wss://` after a `TlsConnector` handshake
//!     (currently NotImplemented in `mod.rs::connect_to`)
//!   - `ServerTls` — inbound `wss://` after a `TlsAcceptor` handshake
//!     (Phase 4.3)
//!
//! All variants implement `AsyncRead + AsyncWrite + Unpin + Send` so
//! they slot into `tokio_tungstenite::client_async` /
//! `tokio_tungstenite::accept_async`.

use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

#[cfg(feature = "wss")]
use tokio_rustls::{client::TlsStream as ClientTlsStream, server::TlsStream as ServerTlsStream};

/// Stream backing a SIP WebSocket connection. Hides whether the
/// underlying transport is plain TCP, client-side TLS, or server-side
/// TLS so the rest of the `ws` module can stay generic over the
/// concrete connection direction.
#[derive(Debug)]
pub enum SipWsStream {
    /// Plain TCP — used for `ws://` in both directions, and as the
    /// client-side `Plain` variant of `MaybeTlsStream` before Phase 4
    /// existed.
    Plain(TcpStream),
    /// Deterministic bounded in-memory stream used by writer cancellation
    /// tests. It is never present in production builds.
    #[cfg(test)]
    Test(tokio::io::DuplexStream),
    /// Client-side TLS, after a `TlsConnector::connect()` handshake.
    /// Built by `WebSocketTransport::connect_to()` for outbound
    /// `wss://` (currently NotImplemented; reserved here so the type
    /// surface is stable).
    #[cfg(feature = "wss")]
    ClientTls(ClientTlsStream<TcpStream>),
    /// Server-side TLS, after a `TlsAcceptor::accept()` handshake.
    /// Built by the WebSocket listener supervisor for inbound `wss://`.
    #[cfg(feature = "wss")]
    ServerTls(ServerTlsStream<TcpStream>),
}

impl AsyncRead for SipWsStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            SipWsStream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(test)]
            SipWsStream::Test(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(feature = "wss")]
            SipWsStream::ClientTls(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(feature = "wss")]
            SipWsStream::ServerTls(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for SipWsStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            SipWsStream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(test)]
            SipWsStream::Test(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(feature = "wss")]
            SipWsStream::ClientTls(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(feature = "wss")]
            SipWsStream::ServerTls(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            SipWsStream::Plain(s) => Pin::new(s).poll_flush(cx),
            #[cfg(test)]
            SipWsStream::Test(s) => Pin::new(s).poll_flush(cx),
            #[cfg(feature = "wss")]
            SipWsStream::ClientTls(s) => Pin::new(s).poll_flush(cx),
            #[cfg(feature = "wss")]
            SipWsStream::ServerTls(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            SipWsStream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(test)]
            SipWsStream::Test(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(feature = "wss")]
            SipWsStream::ClientTls(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(feature = "wss")]
            SipWsStream::ServerTls(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}
