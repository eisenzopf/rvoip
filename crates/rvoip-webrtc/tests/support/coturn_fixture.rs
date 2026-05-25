//! G9a — coturn test fixture.
//!
//! Spins up a coturn TURN server in a Docker container on demand and
//! returns an [`IceServerConfig`] pointing at it. Gracefully **skips**
//! (returns `None`) when the `docker` CLI isn't available or the image
//! pull fails — so the test still runs green on CI without Docker.
//!
//! Usage in a test:
//!
//! ```ignore
//! use crate::support::coturn_fixture::CoturnFixture;
//!
//! let Some(coturn) = CoturnFixture::start().await else {
//!     eprintln!("skipped: docker unavailable");
//!     return;
//! };
//! let ice_config = coturn.ice_config();
//! // ... use ice_config in WebRtcConfig::ice_servers ...
//! drop(coturn); // RAII teardown: docker stop + rm
//! ```
//!
//! The fixture intentionally does not add a `bollard` workspace dependency
//! — it shells out to the `docker` CLI to keep dep bloat zero. Tests that
//! need richer Docker control should add `bollard` to dev-deps and replace
//! this fixture.

use std::time::Duration;

use rvoip_webrtc::IceServerConfig;
use tokio::process::Command;

const COTURN_IMAGE: &str = "coturn/coturn:latest";
const TURN_USERNAME: &str = "webrtctest";
const TURN_PASSWORD: &str = "turnsecret";

pub struct CoturnFixture {
    container_id: String,
    /// Host port mapped to coturn's 3478/udp.
    host_port: u16,
}

impl CoturnFixture {
    /// Try to start coturn in a fresh container. Returns `None` when Docker
    /// isn't reachable or the image pull/run fails.
    pub async fn start() -> Option<Self> {
        // 1. Is docker on PATH?
        if Command::new("docker")
            .arg("--version")
            .output()
            .await
            .ok()
            .map(|o| o.status.success())
            != Some(true)
        {
            return None;
        }

        // 2. Pull a free host port. The container will listen on 3478 inside.
        let host_port = pick_free_port().await?;

        // 3. Start the container in detached mode with credential-based auth.
        let realm = "rvoip-test";
        let user = format!("{TURN_USERNAME}:{TURN_PASSWORD}");
        let cmd_args = vec![
            "run".to_string(),
            "-d".to_string(),
            "--rm".to_string(),
            "-p".to_string(),
            format!("{host_port}:3478/udp"),
            COTURN_IMAGE.to_string(),
            "-n".to_string(),
            "--realm".to_string(),
            realm.to_string(),
            "--user".to_string(),
            user,
            "--no-tls".to_string(),
            "--no-dtls".to_string(),
            "--no-cli".to_string(),
            "--no-stun".to_string(),
        ];
        let out = Command::new("docker").args(&cmd_args).output().await.ok()?;
        if !out.status.success() {
            eprintln!(
                "coturn docker run failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            return None;
        }
        let container_id = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if container_id.is_empty() {
            return None;
        }

        // 4. Give coturn a moment to bind.
        tokio::time::sleep(Duration::from_millis(500)).await;

        Some(Self {
            container_id,
            host_port,
        })
    }

    /// Produce a ready-to-use `IceServerConfig` for this coturn instance.
    pub fn ice_config(&self) -> IceServerConfig {
        IceServerConfig::turn(
            format!("turn:127.0.0.1:{}?transport=udp", self.host_port),
            TURN_USERNAME,
            TURN_PASSWORD,
        )
    }

    pub fn host_port(&self) -> u16 {
        self.host_port
    }
}

impl Drop for CoturnFixture {
    fn drop(&mut self) {
        let id = self.container_id.clone();
        // Best-effort teardown — synchronous std::process::Command so it
        // runs even when the tokio runtime is shutting down.
        let _ = std::process::Command::new("docker")
            .args(["stop", &id])
            .output();
    }
}

async fn pick_free_port() -> Option<u16> {
    let s = tokio::net::TcpListener::bind("127.0.0.1:0").await.ok()?;
    let port = s.local_addr().ok()?.port();
    drop(s);
    Some(port)
}
