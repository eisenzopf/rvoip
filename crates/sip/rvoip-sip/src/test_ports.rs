//! Test-only: start a coordinator on a free loopback SIP port.
//!
//! Binding `127.0.0.1:0` to learn a free port, dropping that socket and
//! handing the port to a coordinator races parallel tests: one of them can
//! bind the port in between, and the coordinator then fails with
//! `EADDRINUSE`. The coordinator cannot be handed a pre-bound socket, so
//! these helpers probe again and retry a bind that lost that race.

use std::future::Future;
use std::sync::Arc;

use crate::api::unified::{Config, UnifiedCoordinator};
use crate::errors::SessionError;

/// Fresh probes before giving up; each one loses only to a parallel bind.
const ATTEMPTS: usize = 32;

/// A loopback UDP port nothing was bound to a moment ago. Another socket can
/// take it before the caller binds it: bind it through a retry.
pub(crate) fn probe_udp_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .and_then(|socket| socket.local_addr())
        .expect("probe a loopback UDP port")
        .port()
}

/// Whether `error` is a bind that lost its port to another socket.
///
/// `SessionError`'s `Debug` and `Display` redact their text, so this reads
/// the variant fields themselves.
pub(crate) fn address_in_use(error: &SessionError) -> bool {
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

/// Run `start` with a freshly probed loopback UDP port until its bind does
/// not lose the port to another socket. Any other failure panics.
pub(crate) async fn on_free_udp_port<T, F, Fut>(mut start: F) -> (T, u16)
where
    F: FnMut(u16) -> Fut,
    Fut: Future<Output = Result<T, SessionError>>,
{
    for _ in 0..ATTEMPTS {
        let port = probe_udp_port();
        match start(port).await {
            Ok(started) => return (started, port),
            Err(error) if address_in_use(&error) => continue,
            Err(error) => panic!("start on 127.0.0.1:{port}: {error:?}"),
        }
    }
    panic!("no free loopback UDP port after {ATTEMPTS} probes")
}

/// A coordinator built from `configure(port)` on a free loopback UDP port.
pub(crate) async fn coordinator_on_free_udp_port(
    configure: impl Fn(u16) -> Config,
) -> (Arc<UnifiedCoordinator>, u16) {
    on_free_udp_port(|port| Box::pin(UnifiedCoordinator::new(configure(port)))).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_coordinator_bind_on_an_occupied_port_is_address_in_use() {
        let held = std::net::UdpSocket::bind("127.0.0.1:0").expect("hold a port");
        let port = held.local_addr().expect("held address").port();
        let error = match Box::pin(UnifiedCoordinator::new(Config::local("occupied", port))).await {
            Ok(_) => panic!("bound a port another socket holds"),
            Err(error) => error,
        };
        let detail = match &error {
            SessionError::InternalError(detail) => detail.clone(),
            other => format!("{other:?}"),
        };
        assert!(address_in_use(&error), "{detail}");
        assert!(!address_in_use(&SessionError::InternalError(
            "transport closed".into()
        )));
    }

    #[tokio::test]
    async fn retries_a_lost_bind_on_a_fresh_probe() {
        let mut attempts = 0;
        let (bound, probed) = on_free_udp_port(|port| {
            attempts += 1;
            let result = if attempts == 1 {
                Err(SessionError::IoError(std::io::Error::from(
                    std::io::ErrorKind::AddrInUse,
                )))
            } else {
                std::net::UdpSocket::bind(("127.0.0.1", port))
                    .map(|socket| socket.local_addr().expect("bound address").port())
                    .map_err(SessionError::IoError)
            };
            std::future::ready(result)
        })
        .await;
        assert_eq!(attempts, 2);
        assert_eq!(bound, probed);
    }
}
