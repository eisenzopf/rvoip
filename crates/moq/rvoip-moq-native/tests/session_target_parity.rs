// SPDX-FileCopyrightText: 2026 Bridgefu contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{path::PathBuf, time::Duration};

use anyhow::Context as _;
use moq_native_ietf::{quic, tls};
use moq_transport::session::{Session, SessionTarget, Transport};
use tokio::time::timeout;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn test_tls() -> anyhow::Result<tls::Config> {
    tls::Args {
        cert: vec![fixture("localhost-cert.pem")],
        key: vec![fixture("localhost-key.pem")],
        root: Vec::new(),
        disable_verify: true,
        ..Default::default()
    }
    .load()
}

async fn assert_target_parity(policy: quic::SubstratePolicy) -> anyhow::Result<Transport> {
    let endpoint = quic::Endpoint::new(quic::Config::new(
        "127.0.0.1:0".parse()?,
        None,
        test_tls()?,
    )?)?;
    let quic::Endpoint { client, server, .. } = endpoint;
    let mut server = server.context("test endpoint did not expose a server")?;
    let server_addr = server.local_addr()?;
    let target: SessionTarget = format!(
        "moqt://localhost:{}/tenant/a%2Fb?token=x%2Fy",
        server_addr.port()
    )
    .parse()?;

    let (client_connection, server_connection) = timeout(TEST_TIMEOUT, async {
        tokio::join!(
            client.connect_target(&target, policy, Some(server_addr)),
            server.accept_connection(),
        )
    })
    .await
    .context("timed out establishing test transport")?;
    let client_connection = client_connection?;
    let server_connection =
        server_connection.context("server stopped before accepting test connection")?;

    assert_eq!(client_connection.negotiated, server_connection.negotiated);
    assert_eq!(client_connection.negotiated.protocol, "moqt-19");
    let substrate = client_connection.negotiated.substrate;

    let (client_setup, server_setup) = timeout(TEST_TIMEOUT, async {
        tokio::join!(
            Session::connect(
                client_connection.session,
                None,
                client_connection.negotiated,
            ),
            Session::accept(
                server_connection.session,
                None,
                server_connection.negotiated,
            ),
        )
    })
    .await
    .context("timed out exchanging SETUP")?;
    let (client_session, _, _) = client_setup?;
    let (server_session, _, _) = server_setup?;

    assert_eq!(client_session.target(), &target);
    assert_eq!(server_session.target(), &target);
    assert_eq!(
        client_session.connection_path(),
        Some("/tenant/a%2Fb?token=x%2Fy")
    );
    assert_eq!(
        server_session.connection_path(),
        Some("/tenant/a%2Fb?token=x%2Fy")
    );
    assert_eq!(client_session.negotiated_transport().protocol, "moqt-19");
    assert_eq!(server_session.negotiated_transport().protocol, "moqt-19");

    Ok(substrate)
}

#[tokio::test]
async fn raw_quic_and_webtransport_have_identical_session_targets() -> anyhow::Result<()> {
    assert_eq!(
        assert_target_parity(quic::SubstratePolicy::RawQuic).await?,
        Transport::RawQuic
    );
    assert_eq!(
        assert_target_parity(quic::SubstratePolicy::WebTransport).await?,
        Transport::WebTransport
    );
    Ok(())
}

#[tokio::test]
async fn auto_negotiates_a_supported_substrate_with_target_parity() -> anyhow::Result<()> {
    assert!(matches!(
        assert_target_parity(quic::SubstratePolicy::Auto).await?,
        Transport::RawQuic | Transport::WebTransport
    ));
    Ok(())
}
