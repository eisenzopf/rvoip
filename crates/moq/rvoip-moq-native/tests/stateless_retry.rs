// SPDX-FileCopyrightText: 2026 Bridgefu contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::time::Duration;

use anyhow::Context;
use moq_native_ietf::{quic, tls};

mod support;

fn development_tls() -> anyhow::Result<tls::Config> {
    support::localhost_server_tls(tls::ClientAuthMode::Disabled, &[])
}

async fn connect(stateless_retry: bool) -> anyhow::Result<u64> {
    let config = quic::Config::new("127.0.0.1:0".parse()?, None, development_tls()?)?
        .with_stateless_retry(stateless_retry);
    let endpoint = quic::Endpoint::new(config)?;
    assert_eq!(endpoint.uses_stateless_retry(), stateless_retry);
    let client = endpoint.client;
    let mut server = endpoint.server.context("missing test server")?;
    let address = server.local_addr()?;
    let target = format!("moqt://localhost:{}/retry", address.port()).parse()?;

    let (client_connection, server_connection) = tokio::join!(
        tokio::time::timeout(
            Duration::from_secs(3),
            client.connect_target(&target, quic::SubstratePolicy::RawQuic, Some(address)),
        ),
        tokio::time::timeout(Duration::from_secs(3), server.accept_connection()),
    );
    client_connection.context("client connection timed out")??;
    server_connection
        .context("server accept timed out")?
        .context("server endpoint closed")?;
    Ok(server.stateless_retries_sent())
}

#[tokio::test]
async fn stateless_retry_is_default_and_validated_retry_connects() -> anyhow::Result<()> {
    assert!(connect(true).await? >= 1);
    Ok(())
}

#[tokio::test]
async fn development_override_accepts_without_retry() -> anyhow::Result<()> {
    assert_eq!(connect(false).await?, 0);
    Ok(())
}

#[tokio::test]
async fn tls_key_logging_is_explicit_and_default_off() -> anyhow::Result<()> {
    let default = quic::Endpoint::new(quic::Config::new(
        "127.0.0.1:0".parse()?,
        None,
        development_tls()?,
    )?)?;
    assert!(!default.tls_key_logging_enabled());

    let diagnostic = quic::Endpoint::new(
        quic::Config::new("127.0.0.1:0".parse()?, None, development_tls()?)?.with_tls_key_log(true),
    )?;
    assert!(diagnostic.tls_key_logging_enabled());
    Ok(())
}
