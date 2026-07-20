//! Hermetic TURN fixture for relay integration tests.
//!
//! The former Docker/coturn fixture could turn a broken relay-only path into
//! a skipped green test when Docker networking was unavailable. This fixture
//! runs the same TURN implementation used by the alpha WebRTC fork's own
//! relay qualification entirely in-process, so startup and relay failures are
//! deterministic test failures.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rvoip_webrtc::IceServerConfig;
use tokio::net::UdpSocket;
use turn_server::auth::{generate_auth_key, AuthHandler};
use turn_server::relay::relay_none::RelayAddressGeneratorNone;
use turn_server::server::config::{ConnConfig, ServerConfig};
use turn_server::server::Server;
use webrtc_util::vnet::net::Net;

pub const TURN_USERNAME: &str = "webrtctest";
pub const TURN_PASSWORD: &str = "turnsecret";
const TURN_REALM: &str = "rvoip-test";

struct TestAuth;

impl AuthHandler for TestAuth {
    fn auth_handle(
        &self,
        username: &str,
        realm: &str,
        _src_addr: SocketAddr,
    ) -> Result<Vec<u8>, turn_server::Error> {
        Ok(generate_auth_key(username, realm, TURN_PASSWORD))
    }
}

pub struct CoturnFixture {
    server: Server,
    host_port: u16,
}

impl CoturnFixture {
    pub async fn start() -> Result<Self, turn_server::Error> {
        let listener = Arc::new(UdpSocket::bind("127.0.0.1:0").await?);
        let host_port = listener.local_addr()?.port();
        let server = Server::new(ServerConfig {
            conn_configs: vec![ConnConfig {
                conn: listener,
                relay_addr_generator: Box::new(RelayAddressGeneratorNone {
                    address: "127.0.0.1".to_owned(),
                    net: Arc::new(Net::new(None)),
                }),
            }],
            realm: TURN_REALM.to_owned(),
            auth_handler: Arc::new(TestAuth),
            channel_bind_timeout: Duration::ZERO,
            alloc_close_notify: None,
        })
        .await?;
        Ok(Self { server, host_port })
    }

    #[allow(dead_code)] // this support module is compiled independently per integration test
    pub fn ice_config(&self) -> IceServerConfig {
        IceServerConfig::turn(
            format!("turn:127.0.0.1:{}?transport=udp", self.host_port),
            TURN_USERNAME,
            TURN_PASSWORD,
        )
    }

    #[allow(dead_code)] // this support module is compiled independently per integration test
    pub fn host_port(&self) -> u16 {
        self.host_port
    }

    pub async fn close(self) -> Result<(), turn_server::Error> {
        self.server.close().await
    }
}
