//! Per-contact Chime TURN credentials. Never cache credentials across contacts
//! and never include endpoints, tokens or response bodies in diagnostics.
use crate::{
    control::ConnectionData,
    errors::{ConnectError, Result},
};
use rvoip_webrtc::IceServerConfig;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnCredentialSource {
    /// Prefer usable JOIN_ACK credentials; fall back to MediaPlacement's URL.
    #[default]
    Auto,
    JoinAck,
    TurnControlUrl,
    /// Use only operator-configured ICE servers.
    Disabled,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConnectTurnConfig {
    pub credential_source: TurnCredentialSource,
    /// Prevent direct Connect media attempts and fail if no relay is usable.
    pub relay_only: bool,
    pub request_timeout_secs: u64,
    /// Optional exact hostname allowlist for the AWS-returned control endpoint.
    /// Empty accepts any HTTPS endpoint returned by StartWebRTCContact.
    pub control_allowed_hosts: Vec<String>,
}
impl Default for ConnectTurnConfig {
    fn default() -> Self {
        Self {
            credential_source: TurnCredentialSource::Auto,
            relay_only: false,
            request_timeout_secs: 10,
            control_allowed_hosts: Vec::new(),
        }
    }
}
impl ConnectTurnConfig {
    pub fn validate(&self) -> Result<()> {
        if !(1..=60).contains(&self.request_timeout_secs)
            || self.control_allowed_hosts.len() > 32
            || self.control_allowed_hosts.iter().any(|host| {
                host.is_empty() || host.len() > 253 || host.contains(['/', ':', '@', '*'])
            })
        {
            return Err(ConnectError::Control(
                "invalid Connect TURN configuration".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct TurnResponse {
    username: String,
    password: String,
    ttl: u64,
    uris: Vec<String>,
}
impl Drop for TurnResponse {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.username.zeroize();
        self.password.zeroize();
    }
}
fn response_servers(response: &TurnResponse) -> Result<Vec<IceServerConfig>> {
    if response.ttl == 0
        || response.username.is_empty()
        || response.password.is_empty()
        || response.username.len() > 8192
        || response.password.len() > 8192
        || response.uris.is_empty()
        || response.uris.len() > 32
        || response
            .uris
            .iter()
            .any(|uri| uri.len() > 8192 || !(uri.starts_with("turn:") || uri.starts_with("turns:")))
    {
        return Err(ConnectError::Signaling(
            "invalid Connect TURN response".into(),
        ));
    }
    Ok(response
        .uris
        .iter()
        .map(|uri| IceServerConfig::turn(uri, &response.username, &response.password))
        .collect())
}
fn has_udp(servers: &[IceServerConfig]) -> bool {
    servers.iter().any(|server| {
        server.username.as_ref().is_some_and(|s| !s.is_empty())
            && server.credential.as_ref().is_some_and(|s| !s.is_empty())
            && server.urls.iter().any(|uri| {
                uri.starts_with("turn:") && (!uri.contains('?') || uri.ends_with("?transport=udp"))
            })
    })
}
fn control_url(connection: &ConnectionData, config: &ConnectTurnConfig) -> Result<url::Url> {
    let url = url::Url::parse(
        connection
            .media_placement
            .turn_control_url
            .as_deref()
            .ok_or(ConnectError::MissingConnectionData("TurnControlUrl"))?,
    )
    .map_err(|_| ConnectError::Signaling("invalid TURN control endpoint".into()))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
        || (!config.control_allowed_hosts.is_empty()
            && !config
                .control_allowed_hosts
                .iter()
                .any(|host| Some(host.as_str()) == url.host_str()))
    {
        return Err(ConnectError::Signaling(
            "TURN control endpoint rejected".into(),
        ));
    }
    Ok(url)
}

async fn fetch(
    connection: &ConnectionData,
    config: &ConnectTurnConfig,
) -> Result<Vec<IceServerConfig>> {
    let url = control_url(connection, config)?;
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let tls = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|_| ConnectError::Signaling("TURN TLS configuration failed".into()))?
    .with_root_certificates(roots)
    .with_no_client_auth();
    let client = reqwest::Client::builder()
        .use_preconfigured_tls(tls)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(config.request_timeout_secs))
        .build()
        .map_err(|_| ConnectError::Signaling("TURN HTTP client failed".into()))?;
    // Match Chime SDK's base attendee token, without an optional modality suffix.
    let token = connection.join_token.split('#').next().unwrap_or_default();
    let mut auth = reqwest::header::HeaderValue::from_str(&format!("_aws_wt_session={token}"))
        .map_err(|_| ConnectError::Signaling("invalid TURN authorization token".into()))?;
    auth.set_sensitive(true);
    let mut response = client
        .post(url)
        .header("X-Chime-Auth-Token", auth)
        .json(&serde_json::json!({"meetingId": connection.meeting_id}))
        .send()
        .await
        .map_err(|_| ConnectError::Signaling("TURN credential request failed".into()))?;
    if !response.status().is_success() {
        return Err(ConnectError::Signaling(format!(
            "TURN credential request rejected (HTTP {})",
            response.status().as_u16()
        )));
    }
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ConnectError::Signaling("TURN response read failed".into()))?
    {
        if bytes.len() + chunk.len() > 65536 {
            return Err(ConnectError::Signaling("TURN response too large".into()));
        }
        bytes.extend_from_slice(&chunk);
    }
    let parsed: TurnResponse = serde_json::from_slice(&bytes)
        .map_err(|_| ConnectError::Signaling("invalid TURN credential response".into()))?;
    response_servers(&parsed)
}

pub(crate) async fn resolve(
    connection: &ConnectionData,
    config: &ConnectTurnConfig,
    join_servers: Vec<IceServerConfig>,
) -> Result<Vec<IceServerConfig>> {
    config.validate()?;
    let servers = match config.credential_source {
        TurnCredentialSource::Disabled => return Ok(Vec::new()),
        TurnCredentialSource::JoinAck => join_servers,
        TurnCredentialSource::Auto if has_udp(&join_servers) => join_servers,
        TurnCredentialSource::Auto | TurnCredentialSource::TurnControlUrl => {
            let result = tokio::time::timeout(
                Duration::from_secs(config.request_timeout_secs),
                fetch(connection, config),
            )
            .await
            .map_err(|_| ConnectError::Timeout("TURN credential request"))
            .and_then(|result| result);
            match result {
                Ok(servers) => servers,
                Err(_)
                    if config.credential_source == TurnCredentialSource::Auto
                        && !config.relay_only =>
                {
                    tracing::warn!("Connect TURN credentials unavailable; configured ICE policy permits direct media");
                    Vec::new()
                }
                Err(error) => return Err(error),
            }
        }
    };
    if config.relay_only && !has_udp(&servers) {
        return Err(ConnectError::Signaling(
            "Connect supplied no usable UDP TURN credentials".into(),
        ));
    }
    tracing::info!(credential_source = ?config.credential_source, relay_only = config.relay_only, ice_server_count = servers.len(), "resolved per-contact Connect TURN configuration");
    Ok(servers)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn connection(endpoint: Option<&str>) -> ConnectionData {
        ConnectionData {
            contact_id: "contact-test".into(),
            participant_id: "participant-test".into(),
            participant_token: "participant-token-test".into(),
            meeting_id: "meeting-test".into(),
            media_region: "region-test".into(),
            attendee_id: "attendee-test".into(),
            join_token: "join-token-test".into(),
            media_placement: crate::control::MediaPlacement {
                signaling_url: "wss://signaling.example.test".into(),
                audio_host_url: "audio.example.test".into(),
                turn_control_url: endpoint.map(str::to_owned),
                audio_fallback_url: None,
                event_ingestion_url: None,
            },
        }
    }

    #[test]
    fn control_endpoint_requires_https_and_honors_customer_allowlist() {
        let mut config = ConnectTurnConfig::default();
        for invalid in [
            "http://control.example.test",
            "https://user:secret@control.example.test",
            "https://control.example.test/#fragment",
        ] {
            assert!(control_url(&connection(Some(invalid)), &config).is_err());
        }
        config.control_allowed_hosts = vec!["allowed.example.test".into()];
        assert!(control_url(
            &connection(Some("https://allowed.example.test/turn")),
            &config
        )
        .is_ok());
        assert!(control_url(
            &connection(Some("https://other.example.test/turn")),
            &config
        )
        .is_err());
    }

    #[tokio::test]
    async fn credential_source_selection_and_fail_closed_policy() {
        let conn = connection(None);
        let mut config = ConnectTurnConfig::default();
        let join = || {
            vec![IceServerConfig::turn(
                "turn:relay.example.test:3478?transport=udp",
                "username-test",
                "password-test",
            )]
        };
        assert!(has_udp(&resolve(&conn, &config, join()).await.unwrap()));
        assert!(resolve(&conn, &config, vec![]).await.unwrap().is_empty());
        config.relay_only = true;
        assert!(resolve(&conn, &config, vec![]).await.is_err());
        config.credential_source = TurnCredentialSource::TurnControlUrl;
        assert!(
            resolve(&conn, &config, join()).await.is_err(),
            "forced URL mode must not silently use JOIN_ACK"
        );
        config.credential_source = TurnCredentialSource::Disabled;
        assert!(resolve(&conn, &config, join()).await.unwrap().is_empty());
        config.credential_source = TurnCredentialSource::JoinAck;
        assert!(has_udp(&resolve(&conn, &config, join()).await.unwrap()));
    }

    #[test]
    fn validates_credentials_without_exposing_them() {
        let mut response = TurnResponse {
            username: "user-private".into(),
            password: "password-private".into(),
            ttl: 60,
            uris: vec!["turn:relay.example.test:3478?transport=udp".into()],
        };
        assert!(has_udp(&response_servers(&response).unwrap()));
        response.ttl = 0;
        let error = response_servers(&response).unwrap_err().to_string();
        assert!(!error.contains("private"));
        response.ttl = 60;
        response.uris = vec!["turns:relay.example.test:443?transport=tcp".into()];
        assert!(!has_udp(&response_servers(&response).unwrap()));
    }
}
