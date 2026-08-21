// SPDX-FileCopyrightText: 2026 Bridgefu contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::time::Duration;

use anyhow::Context as _;
use moq_native_ietf::{quic, tls};
use moq_transport::session::SessionTarget;
use moq_transport::{
    coding::TrackNamespace,
    serve,
    session::{Publisher, Subscriber},
};
use tokio::time::timeout;

mod common;

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::test]
async fn track_status_round_trips_on_independent_request_streams() -> anyhow::Result<()> {
    let identity = common::localhost_server_identity()?;
    let tls = tls::Args {
        cert: vec![identity.cert],
        key: vec![identity.key],
        root: Vec::new(),
        disable_verify: true,
        ..Default::default()
    }
    .load()?;
    let endpoint = quic::Endpoint::new(quic::Config::new("127.0.0.1:0".parse()?, None, tls)?)?;
    let quic::Endpoint { client, server, .. } = endpoint;
    let mut server = server.context("test endpoint did not expose a server")?;
    let server_addr = server.local_addr()?;
    let target: SessionTarget = format!("moqt://localhost:{}", server_addr.port()).parse()?;

    let (client_connection, server_connection) = tokio::join!(
        timeout(
            TEST_TIMEOUT,
            client.connect_target(&target, quic::SubstratePolicy::RawQuic, Some(server_addr)),
        ),
        timeout(TEST_TIMEOUT, server.accept_connection()),
    );
    let client_connection = client_connection??;
    let server_connection =
        server_connection?.context("server stopped before accepting test connection")?;

    let (client_setup, server_setup) = tokio::join!(
        Subscriber::connect(client_connection.session, client_connection.negotiated),
        Publisher::accept(server_connection.session, server_connection.negotiated),
    );
    let (client_session, mut subscriber) = client_setup?;
    let (server_session, mut publisher) = server_setup?;

    let client_run = tokio::spawn(client_session.run());
    let server_run = tokio::spawn(server_session.run());

    let namespace = TrackNamespace::from_utf8_path("live");
    let (_track_writer, track_reader) = serve::Track::new(namespace.clone(), "audio").produce();
    let responder = tokio::spawn(async move {
        for _ in 0..2 {
            let request = timeout(TEST_TIMEOUT, publisher.track_status_requested())
                .await
                .context("timed out waiting for TRACK_STATUS")?
                .context("TRACK_STATUS request queue closed")?;
            request.respond_ok(&track_reader)?;
        }
        Ok::<(), anyhow::Error>(())
    });

    let first = timeout(
        TEST_TIMEOUT,
        subscriber.track_status_query(&namespace, "audio"),
    )
    .await??;
    assert_eq!(first.id, 0);

    let second = timeout(
        TEST_TIMEOUT,
        subscriber.track_status_query(&namespace, "audio"),
    )
    .await??;
    assert_eq!(second.id, 2);

    responder.await??;
    assert!(
        !client_run.is_finished(),
        "client session ended after TRACK_STATUS request-stream completion"
    );
    assert!(
        !server_run.is_finished(),
        "server session ended after TRACK_STATUS request-stream completion"
    );

    client_run.abort();
    server_run.abort();
    let _ = client_run.await;
    let _ = server_run.await;

    Ok(())
}
