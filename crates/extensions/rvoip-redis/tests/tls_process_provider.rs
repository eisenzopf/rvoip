//! With both rustls providers compiled (the `aws_lc_rs` dev-dependency
//! feature) and no process-level default installed, a `rediss://` connection
//! works: rvoip-redis installs ring as the process default before redis
//! builds its TLS configuration on that default.
//!
//! The server is a local TLS listener with a self-signed certificate the
//! client is told to trust, answering RESP commands (`HLEN` with 0, anything
//! else with `OK`), so no Redis is needed. Its own TLS configuration names
//! ring explicitly, so the test never installs a provider itself.
//! Process-level provider state is global, so this has its own test binary.

use std::sync::{Arc, Mutex};

use rustls::pki_types::PrivateKeyDer;
use rvoip_redis::{RedisAuthConfig, RedisAuthProvider, RedisAuthRuntimeConfig, RedisAuthTlsConfig};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

/// Read one RESP array command (`*N` of `$len` bulk strings).
async fn read_command<R: AsyncBufRead + Unpin>(reader: &mut R) -> Option<Vec<String>> {
    let mut line = String::new();
    if reader.read_line(&mut line).await.ok()? == 0 {
        return None;
    }
    let count: usize = line.trim_end().strip_prefix('*')?.parse().ok()?;
    let mut parts = Vec::with_capacity(count);
    for _ in 0..count {
        line.clear();
        reader.read_line(&mut line).await.ok()?;
        let len: usize = line.trim_end().strip_prefix('$')?.parse().ok()?;
        let mut data = vec![0; len + 2];
        reader.read_exact(&mut data).await.ok()?;
        data.truncate(len);
        parts.push(String::from_utf8_lossy(&data).into_owned());
    }
    Some(parts)
}

async fn serve_resp_over_tls(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    commands: Arc<Mutex<Vec<String>>>,
) {
    while let Ok((tcp, _)) = listener.accept().await {
        let acceptor = acceptor.clone();
        let commands = commands.clone();
        tokio::spawn(async move {
            let Ok(tls) = acceptor.accept(tcp).await else {
                return;
            };
            let (read, mut write) = tokio::io::split(tls);
            let mut reader = BufReader::new(read);
            while let Some(command) = read_command(&mut reader).await {
                let name = command
                    .first()
                    .map(|name| name.to_ascii_uppercase())
                    .unwrap_or_default();
                let reply: &[u8] = if name == "HLEN" {
                    b":0\r\n"
                } else {
                    b"+OK\r\n"
                };
                commands.lock().unwrap().push(name);
                if write.write_all(reply).await.is_err() {
                    return;
                }
            }
        });
    }
}

#[tokio::test]
async fn rediss_without_a_process_provider_installs_ring_and_connects() {
    assert!(
        rustls::crypto::CryptoProvider::get_default().is_none(),
        "no process-level provider is installed at the start"
    );

    let certificate =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .unwrap();
    let server_config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![certificate.cert.der().clone()],
        PrivateKeyDer::Pkcs8(certificate.signing_key.serialize_der().into()),
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let commands = Arc::new(Mutex::new(Vec::new()));
    tokio::spawn(serve_resp_over_tls(
        listener,
        TlsAcceptor::from(Arc::new(server_config)),
        commands.clone(),
    ));

    let provider = RedisAuthProvider::from_config_with_runtime_and_tls(
        RedisAuthConfig::new(format!("rediss://127.0.0.1:{port}")),
        RedisAuthRuntimeConfig::default(),
        RedisAuthTlsConfig::new().with_root_certificate_pem(certificate.cert.pem()),
    )
    .expect("client configuration");
    provider
        .auth_rate_limit_cardinality()
        .await
        .expect("rediss connects and answers with no process-level provider installed");

    let hlen = commands
        .lock()
        .unwrap()
        .iter()
        .filter(|name| *name == "HLEN")
        .count();
    assert_eq!(
        hlen, 3,
        "the three HLEN commands crossed the TLS connection"
    );
    let installed = rustls::crypto::CryptoProvider::get_default().expect("ring installed");
    assert_eq!(format!("{:?}", installed.key_provider), "Ring");
}
