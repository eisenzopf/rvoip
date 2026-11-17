//! With both rustls providers compiled (the `aws_lc_rs` dev-dependency
//! feature) and no process-level default installed, a `rediss://` connection
//! attempt fails cleanly instead of panicking: rvoip-redis installs ring as the
//! process default before redis builds its TLS configuration on it.
//!
//! The "server" accepts TCP and closes it, so redis gets as far as building
//! its rustls configuration and starting the handshake. No Redis is needed.
//! Process-level provider state is global, so this has its own test binary.

use std::time::Duration;

use rvoip_redis::{RedisAuthConfig, RedisAuthProvider, RedisAuthRuntimeConfig};

#[tokio::test]
async fn rediss_without_a_process_provider_installs_ring_and_fails_cleanly() {
    assert!(rustls::crypto::CryptoProvider::get_default().is_none());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            drop(socket);
        }
    });

    let runtime = RedisAuthRuntimeConfig {
        connection_timeout: Duration::from_millis(500),
        response_timeout: Duration::from_millis(500),
        operation_timeout: Duration::from_secs(2),
        retry_attempts: 0,
        ..RedisAuthRuntimeConfig::default()
    };
    let provider = RedisAuthProvider::from_config_with_runtime(
        RedisAuthConfig::new(format!("rediss://localhost:{port}")),
        runtime,
    )
    .expect("client configuration");
    let result = provider.auth_rate_limit_cardinality().await;
    assert!(result.is_err(), "the closed TLS connection is an error");
    let installed = rustls::crypto::CryptoProvider::get_default().expect("ring installed");
    assert_eq!(format!("{:?}", installed.key_provider), "Ring");
}
