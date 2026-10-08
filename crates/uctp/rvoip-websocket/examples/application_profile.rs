//! Minimal authenticated application control, independent of a media Session.
//! Run with --server, or --client ws://127.0.0.1:<printed-port>.
use std::{error::Error, sync::Arc, time::Duration};

use rvoip_auth_core::{
    AuthenticatedPrincipal, AuthenticationMethod, BearerAuthError, BearerValidator,
};
use rvoip_core::{adapter::ConnectionAdapter, IdentityAssurance, IdentityId, Orchestrator};
use rvoip_uctp::{
    application::{ApplicationContext, ApplicationError, ApplicationHandler},
    envelope::UctpEnvelope,
    types::MessageType,
};
use rvoip_websocket::{UctpWsAdapter, UctpWsClient, UctpWsConfig};
use serde_json::json;
use subtle::ConstantTimeEq;
use tokio::{net::TcpListener, sync::mpsc};
use url::Url;

const PROFILE: &str = "example.echo/1";
const SCOPE: &str = "example:echo";

// One explicitly configured development credential; never accepts any token.
struct DemoBearer(String);

#[async_trait::async_trait]
impl BearerValidator for DemoBearer {
    async fn validate(&self, token: &str) -> Result<IdentityAssurance, BearerAuthError> {
        self.validate_principal(token).await.map(|p| p.assurance)
    }

    async fn validate_principal(
        &self,
        token: &str,
    ) -> Result<AuthenticatedPrincipal, BearerAuthError> {
        // Constant-time comparison so response timing does not reveal how many
        // leading bytes of a guessed credential matched. (`ct_eq` on slices of
        // different lengths returns false immediately; only length can leak.)
        if !bool::from(token.as_bytes().ct_eq(self.0.as_bytes())) {
            return Err(BearerAuthError::Invalid(
                "invalid example credential".into(),
            ));
        }
        Ok(AuthenticatedPrincipal {
            subject: "example-user".into(),
            tenant: Some("example-tenant".into()),
            issuer: Some("local-example".into()),
            scopes: vec![SCOPE.into()],
            expires_at: None,
            method: AuthenticationMethod::Bearer,
            assurance: IdentityAssurance::UserAuthorized {
                identity: IdentityId::from_string("id_example"),
                user_id: IdentityId::from_string("id_example"),
                scopes: vec![SCOPE.into()],
            },
        })
    }
}

struct Echo;

#[async_trait::async_trait]
impl ApplicationHandler for Echo {
    fn profile(&self) -> &'static str {
        PROFILE
    }

    fn required_scope(&self) -> &'static str {
        SCOPE
    }

    async fn handle(
        &self,
        context: ApplicationContext,
        request: UctpEnvelope,
    ) -> Result<UctpEnvelope, ApplicationError> {
        // Scope is checked by the coordinator; object membership is the host's
        // responsibility. This example grants no Conversation/media access.
        if context.principal.tenant.as_deref() != Some("example-tenant")
            || context.principal.subject != "example-user"
            || request.cid.is_some()
            || request.sid.is_some()
            || request.connid.is_some()
        {
            return Err(ApplicationError::new(
                403,
                "example resource not authorized",
            ));
        }
        if request.msg_type != MessageType::Unknown("example.echo".into()) {
            return Err(ApplicationError::new(501, "unsupported example operation"));
        }
        let text = request
            .payload
            .get("text")
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty() && s.len() <= 1024)
            .ok_or_else(|| ApplicationError::new(400, "text must contain 1..1024 bytes"))?;
        Ok(UctpEnvelope::new(
            MessageType::Ack,
            json!({"profile":PROFILE,"text":text,"by":context.principal.subject}),
        ))
    }
    // Default replay rejects duplicate IDs. This example has no effect store.
}

async fn server(token: String) -> Result<(), Box<dyn Error>> {
    // Loopback-only: use a real validator and WSS in a remote application.
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let orchestrator = Orchestrator::new(Default::default());
    let adapter = UctpWsAdapter::new(
        UctpWsConfig::new(listener, Arc::new(DemoBearer(token)))
            .with_application_handler(Arc::new(Echo))
            .with_orchestrator(Arc::clone(&orchestrator)),
    )
    .await?;
    orchestrator.register(adapter as Arc<dyn ConnectionAdapter>)?;
    println!("ws://{address}");
    tokio::signal::ctrl_c().await?;
    Ok(())
}

async fn receive(
    inbound: &mut mpsc::Receiver<UctpEnvelope>,
    request: &UctpEnvelope,
) -> Result<UctpEnvelope, Box<dyn Error>> {
    let reply = tokio::time::timeout(Duration::from_secs(5), inbound.recv())
        .await?
        .ok_or("server disconnected")?;
    if reply.in_reply_to.as_deref() != Some(request.id.as_str()) {
        return Err("uncorrelated response".into());
    }
    Ok(reply)
}

async fn client(url: &str, token: String) -> Result<(), Box<dyn Error>> {
    let url = Url::parse(url)?;
    if url.scheme() != "ws"
        || !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("example client requires a loopback WebSocket URL without userinfo".into());
    }
    let client = UctpWsClient::connect(&url).await?;
    let mut inbound = client.take_inbound().ok_or("missing inbound stream")?;
    let hello = UctpEnvelope::new(
        MessageType::AuthHello,
        json!({"device":{"id":"dev_example","kind":"desktop","platform":"rust","sdk_version":"example/1"},
            "auth_methods":["bearer"],"capabilities":{"application_profiles":[PROFILE]}}),
    );
    client.send(hello.clone()).await?;
    let challenge = receive(&mut inbound, &hello).await?;
    if challenge.msg_type != MessageType::AuthChallenge
        || !challenge.payload["accepted_methods"]
            .as_array()
            .is_some_and(|methods| methods.iter().any(|method| method == "bearer"))
        || !challenge.payload["server_capabilities"]["application_profiles"]
            .as_array()
            .is_some_and(|profiles| profiles.iter().any(|p| p == PROFILE))
    {
        return Err("server did not advertise example.echo/1".into());
    }
    let auth = UctpEnvelope::new(
        MessageType::AuthResponse,
        json!({"method":"bearer","credential":token}),
    )
    .with_in_reply_to(challenge.id);
    client.send(auth.clone()).await?;
    if receive(&mut inbound, &auth).await?.msg_type != MessageType::AuthSession {
        return Err("example authentication failed".into());
    }
    let command = UctpEnvelope::new(
        MessageType::Unknown("example.echo".into()),
        json!({"profile":PROFILE,"text":"Hello through UCTP"}),
    );
    client.send(command.clone()).await?;
    let reply = receive(&mut inbound, &command).await?;
    if reply.msg_type != MessageType::Ack || reply.payload["text"] != "Hello through UCTP" {
        return Err("example echo failed".into());
    }
    println!("Authenticated, correlated echo: {}", reply.payload["text"]);
    client.send(command.clone()).await?;
    let replay = receive(&mut inbound, &command).await?;
    if replay.msg_type != MessageType::Error || replay.payload["code"] != 409 {
        return Err("duplicate command was not refused".into());
    }
    println!("Duplicate ID refused with 409; no durable replay store installed");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let token = std::env::var("RVOIP_EXAMPLE_TOKEN")
        .ok()
        .filter(|token| !token.trim().is_empty())
        .ok_or("set RVOIP_EXAMPLE_TOKEN to a development credential")?;
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [mode] if mode == "--server" => server(token).await,
        [mode, url] if mode == "--client" => client(url, token).await,
        _ => Err("usage: application_profile --server | --client ws://127.0.0.1:<port>".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn context() -> ApplicationContext {
        ApplicationContext {
            principal: DemoBearer("configured".into())
                .validate_principal("configured")
                .await
                .unwrap(),
            outbound: mpsc::channel(1).0,
            closed: Default::default(),
        }
    }

    #[tokio::test]
    async fn example_accepts_only_its_configured_credential() {
        let bearer = DemoBearer("configured".into());
        for wrong in ["wrong", "", "configure", "configured!", "Configured"] {
            assert!(bearer.validate_principal(wrong).await.is_err(), "{wrong:?}");
        }
        assert!(bearer
            .validate_principal("configured")
            .await
            .unwrap()
            .has_scope(SCOPE));
    }

    #[tokio::test]
    async fn echo_validates_payload_and_does_not_grant_resource_access() {
        let command = UctpEnvelope::new(
            MessageType::Unknown("example.echo".into()),
            json!({"profile":PROFILE,"text":"hello"}),
        );
        assert_eq!(
            Echo.handle(context().await, command.clone())
                .await
                .unwrap()
                .payload["text"],
            "hello"
        );
        assert_eq!(
            Echo.handle(context().await, command.with_cid("foreign"))
                .await
                .unwrap_err()
                .code,
            403
        );
        let invalid = UctpEnvelope::new(
            MessageType::Unknown("example.echo".into()),
            json!({"profile":PROFILE}),
        );
        assert_eq!(
            Echo.handle(context().await, invalid)
                .await
                .unwrap_err()
                .code,
            400
        );
    }

    #[tokio::test]
    async fn duplicate_requests_require_a_store_and_unsupported_operations_are_explicit() {
        let request = UctpEnvelope::new(MessageType::MessageSend, json!({"profile":PROFILE}));
        assert_eq!(
            Echo.replay(context().await, request.clone())
                .await
                .unwrap_err()
                .code,
            409
        );
        assert_eq!(
            Echo.handle(context().await, request)
                .await
                .unwrap_err()
                .code,
            501
        );
    }
}
