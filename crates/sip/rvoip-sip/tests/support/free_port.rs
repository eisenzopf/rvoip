//! Start a SIP listener on a free loopback port.
//!
//! Binding `127.0.0.1:0` to learn a free port, dropping that socket and
//! handing the port to a coordinator races parallel tests: one of them can
//! bind the port in between, and the coordinator then fails with
//! `EADDRINUSE`. A coordinator cannot be handed a pre-bound socket, so
//! [`retry_address_in_use`] probes again and retries a bind that lost that
//! race. Files that need only this include it with
//! `#[path = "support/free_port.rs"] mod free_port;`.

#![allow(dead_code)]

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use rvoip_sip::api::unified::{Config, UnifiedCoordinator};
use rvoip_sip::errors::SessionError;
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

/// How long a TLS connect and handshake to a just-started listener may take.
const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(5);

/// Fresh probes before giving up; each one loses only to a parallel bind.
pub const ATTEMPTS: usize = 32;

/// A loopback UDP port nothing was bound to a moment ago. Another socket can
/// take it before the caller binds it: bind it inside [`retry_address_in_use`].
pub fn probe_udp_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .and_then(|socket| socket.local_addr())
        .expect("probe a loopback UDP port")
        .port()
}

/// A loopback TCP port nothing was bound to a moment ago. Another socket can
/// take it before the caller binds it: bind it inside [`retry_address_in_use`].
pub fn probe_tcp_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("probe a loopback TCP port")
        .port()
}

/// Whether `error` is a bind that lost its port to another socket.
///
/// `SessionError`'s `Debug` and `Display` redact their text, so this reads
/// the variant fields themselves.
pub fn address_in_use(error: &SessionError) -> bool {
    let detail = match error {
        SessionError::IoError(error) => return error.kind() == std::io::ErrorKind::AddrInUse,
        SessionError::InternalError(detail)
        | SessionError::NetworkError(detail)
        | SessionError::DialogError(detail)
        | SessionError::ConfigurationError(detail)
        | SessionError::ConfigError(detail)
        | SessionError::Other(detail) => detail,
        _ => return false,
    };
    // The transport manager logs a failed bind and starts with whatever
    // did bind, so a lost UDP port surfaces as "no default transport"
    // rather than the OS error.
    [
        "Address already in use",
        "AddrInUse",
        "os error 98",
        "os error 48",
        "os error 10048",
        "has no default transport",
    ]
    .iter()
    .any(|needle| detail.contains(needle))
}

/// Run `start` (which probes its own ports) until its bind does not lose a
/// port to another socket. Any other failure panics with `what`. The lib's
/// `test_ports` tests check this classification against a real lost bind.
pub async fn retry_address_in_use<T, F, Fut>(what: &str, mut start: F) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, SessionError>>,
{
    for _ in 0..ATTEMPTS {
        match start().await {
            Ok(started) => return started,
            Err(error) if address_in_use(&error) => continue,
            Err(error) => panic!("{what}: {error:?}"),
        }
    }
    panic!("{what}: no free loopback port after {ATTEMPTS} probes")
}

/// Start a coordinator from `configure(tls_port)` on a freshly probed TLS port
/// and return it with a TLS connection to its listener.
///
/// A coordinator whose TLS listener lost its port still starts (a partial
/// transport start is not an error), so the bind outcome is not enough. Only
/// this coordinator's listener can complete a handshake that trusts `root`,
/// the run's freshly generated identity, so a refused connection or a failed
/// handshake means the port was lost: the coordinator is shut down and the
/// start retried on fresh ports.
pub async fn start_with_tls_connection(
    what: &str,
    root: rustls::pki_types::CertificateDer<'static>,
    configure: impl Fn(u16) -> Config,
) -> (Arc<UnifiedCoordinator>, TlsStream<TcpStream>, u16) {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(root).expect("trust the test identity");
    let client = Arc::new(
        rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("TLS protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth(),
    );
    retry_address_in_use(what, || {
        let tls_port = probe_tcp_port();
        let config = configure(tls_port);
        let client = Arc::clone(&client);
        async move {
            let coordinator = Box::pin(UnifiedCoordinator::new(config)).await?;
            match tls_connect(client, tls_port).await {
                Ok(stream) => Ok((coordinator, stream, tls_port)),
                Err(error) => {
                    coordinator.shutdown();
                    Err(SessionError::IoError(std::io::Error::new(
                        std::io::ErrorKind::AddrInUse,
                        format!("the TLS listener does not own 127.0.0.1:{tls_port}: {error}"),
                    )))
                }
            }
        }
    })
    .await
}

async fn tls_connect(
    client: Arc<rustls::ClientConfig>,
    port: u16,
) -> std::io::Result<TlsStream<TcpStream>> {
    let server = rustls::pki_types::ServerName::try_from("localhost").expect("server name");
    tokio::time::timeout(HANDSHAKE_DEADLINE, async {
        let tcp = TcpStream::connect(("127.0.0.1", port)).await?;
        tokio_rustls::TlsConnector::from(client)
            .connect(server, tcp)
            .await
    })
    .await
    .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "TLS handshake deadline"))?
}
