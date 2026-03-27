//! Stream abstraction for WebSocket transport.
//!
//! Provides a unified stream type that can be either a plain TCP stream
//! or a TLS-wrapped TCP stream (for WSS). This is needed because
//! `tokio_tungstenite::MaybeTlsStream::Rustls` only supports client-side
//! TLS streams, but our listener produces server-side TLS streams.

use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

/// A stream that is either plain TCP or TLS over TCP (server or client side).
///
/// Unlike `tokio_tungstenite::MaybeTlsStream`, this enum supports
/// both server-side and client-side TLS streams from `tokio_rustls`.
pub enum WsStream {
    /// Plain (unencrypted) TCP stream — used for `ws://`.
    Plain(TcpStream),
    /// Server-side TLS-encrypted TCP stream — used for `wss://` listener.
    #[cfg(feature = "tls")]
    Tls(tokio_rustls::server::TlsStream<TcpStream>),
    /// Client-side TLS-encrypted TCP stream — used for `wss://` client connections.
    #[cfg(feature = "tls")]
    TlsClient(tokio_rustls::client::TlsStream<TcpStream>),
}

impl std::fmt::Debug for WsStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WsStream::Plain(_) => write!(f, "WsStream::Plain"),
            #[cfg(feature = "tls")]
            WsStream::Tls(_) => write!(f, "WsStream::Tls(server)"),
            #[cfg(feature = "tls")]
            WsStream::TlsClient(_) => write!(f, "WsStream::TlsClient"),
        }
    }
}

impl AsyncRead for WsStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            WsStream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(feature = "tls")]
            WsStream::Tls(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(feature = "tls")]
            WsStream::TlsClient(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for WsStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        match self.get_mut() {
            WsStream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(feature = "tls")]
            WsStream::Tls(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(feature = "tls")]
            WsStream::TlsClient(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        match self.get_mut() {
            WsStream::Plain(s) => Pin::new(s).poll_flush(cx),
            #[cfg(feature = "tls")]
            WsStream::Tls(s) => Pin::new(s).poll_flush(cx),
            #[cfg(feature = "tls")]
            WsStream::TlsClient(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        match self.get_mut() {
            WsStream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(feature = "tls")]
            WsStream::Tls(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(feature = "tls")]
            WsStream::TlsClient(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

impl Unpin for WsStream {}
