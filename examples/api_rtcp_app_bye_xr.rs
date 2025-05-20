// Configure server
let server_config = ServerConfigBuilder::new()
    .local_address("127.0.0.1:0".parse().unwrap())
    .rtcp_mux(true)
    .security_config(rvoip_rtp_core::api::server::security::ServerSecurityConfig {
        security_mode: rvoip_rtp_core::api::common::config::SecurityMode::None,
        ..Default::default()
    })
    .build()
    .unwrap();

// Configure client
let client_config = ClientConfigBuilder::new()
    .remote_address(server_addr)
    .rtcp_mux(true)
    .security_config(rvoip_rtp_core::api::client::security::ClientSecurityConfig {
        security_mode: rvoip_rtp_core::api::common::config::SecurityMode::None,
        ..Default::default()
    })
    .build(); 