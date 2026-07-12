//! Network signaling ownership is adapter-wide, not protocol-local.

#![cfg(all(feature = "signaling-whip", feature = "signaling-ws"))]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use rvoip_auth_core::{AuthenticatedPrincipal, AuthenticationMethod};
use rvoip_core::adapter::{AdapterEvent, ConnectionAdapter};
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::ConnectionId;
use rvoip_webrtc::peer::{PeerRole, RvoipPeerConnection};
use rvoip_webrtc::signaling::auth::{AuthContext, AuthRejection, WhipAuthHook, WsAuthHook};
use rvoip_webrtc::{WebRtcConfig, WebRtcServer, WebRtcServerBuilder};
use tokio_tungstenite::tungstenite::Message;

struct PrincipalTokenAuth;

impl PrincipalTokenAuth {
    fn authenticate_token(token: Option<&str>) -> Result<AuthContext, AuthRejection> {
        let (issuer, tenant, expired) = match token {
            Some("owner") => ("issuer-a", "tenant-a", false),
            Some("expired") => ("issuer-a", "tenant-a", true),
            Some("other-issuer") => ("issuer-b", "tenant-a", false),
            Some("other-tenant") => ("issuer-a", "tenant-b", false),
            _ => {
                return Err(AuthRejection::Unauthorized {
                    www_authenticate: "Bearer realm=\"ownership-test\"".into(),
                })
            }
        };
        let principal = AuthenticatedPrincipal {
            // Intentionally identical across all tokens: subject-only
            // ownership would incorrectly authorize every one of them.
            subject: "shared-subject".into(),
            tenant: Some(tenant.into()),
            scopes: vec![
                "whip:publish".into(),
                "whep:subscribe".into(),
                "webrtc:connect".into(),
            ],
            issuer: Some(issuer.into()),
            expires_at: expired.then(|| chrono::Utc::now() - chrono::Duration::seconds(1)),
            method: AuthenticationMethod::Bearer,
            assurance: IdentityAssurance::Anonymous,
        };
        Ok(AuthContext {
            subject: principal.subject.clone(),
            scopes: principal.scopes.clone(),
            session_hint: (token == Some("owner")).then(|| "ws-attachment".into()),
            principal: Some(principal),
        })
    }
}

#[async_trait]
impl WhipAuthHook for PrincipalTokenAuth {
    async fn authenticate(
        &self,
        _method: &str,
        _path: &str,
        bearer: Option<&str>,
        _peer_addr: SocketAddr,
    ) -> Result<AuthContext, AuthRejection> {
        Self::authenticate_token(bearer)
    }
}

#[async_trait]
impl WsAuthHook for PrincipalTokenAuth {
    async fn authenticate(
        &self,
        subprotocols: &[String],
        query_token: Option<&str>,
        _peer_addr: SocketAddr,
    ) -> Result<AuthContext, AuthRejection> {
        let protocol_token = subprotocols
            .iter()
            .find_map(|value| value.strip_prefix("token."));
        Self::authenticate_token(protocol_token.or(query_token))
    }
}

async fn start_server() -> WebRtcServer {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let auth = Arc::new(PrincipalTokenAuth);
    WebRtcServerBuilder::new(WebRtcConfig::loopback())
        .with_whip("127.0.0.1:0")
        .with_ws("127.0.0.1:0")
        .with_whip_auth(auth.clone())
        .with_ws_auth(auth)
        .build()
        .await
        .expect("server")
}

async fn fresh_offer() -> String {
    let peer = Arc::new(
        RvoipPeerConnection::new(&WebRtcConfig::loopback(), PeerRole::Offerer)
            .await
            .expect("offerer"),
    );
    peer.add_local_audio_track().await.expect("audio track");
    peer.create_offer_and_gather().await.expect("offer")
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("http client")
}

async fn next_event(events: &mut tokio::sync::mpsc::Receiver<AdapterEvent>) -> AdapterEvent {
    tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("adapter event timeout")
        .expect("adapter event channel closed")
}

async fn create_owned_whip_route(server: &WebRtcServer, offer: &str) -> (String, ConnectionId) {
    let address = server.whip_addr().expect("WHIP address");
    let response = http()
        .post(format!("http://{address}/whip/ownership"))
        .header("authorization", "Bearer owner")
        .header("content-type", "application/sdp")
        .body(offer.to_owned())
        .send()
        .await
        .expect("WHIP POST");
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let location = response
        .headers()
        .get("location")
        .expect("location")
        .to_str()
        .expect("location text")
        .to_owned();
    let id = ConnectionId::from_string(
        location
            .rsplit('/')
            .next()
            .expect("connection id")
            .to_owned(),
    );
    (format!("http://{address}{location}"), id)
}

#[tokio::test]
async fn issuer_tenant_and_subject_all_participate_in_update_delete_ownership() {
    let server = start_server().await;
    let mut events = server
        .adapter()
        .try_subscribe_events()
        .expect("adapter events");
    let offer = fresh_offer().await;

    let address = server.whip_addr().expect("WHIP address");
    let expired = http()
        .post(format!("http://{address}/whip/expired"))
        .header("authorization", "Bearer expired")
        .header("content-type", "application/sdp")
        .body(offer.clone())
        .send()
        .await
        .expect("expired principal POST");
    assert_eq!(expired.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert!(server.adapter().routes().is_empty());

    let (route_url, connection_id) = create_owned_whip_route(&server, &offer).await;
    match next_event(&mut events).await {
        AdapterEvent::InboundConnection { connection } => {
            assert_eq!(connection.id, connection_id);
        }
        other => panic!("expected inbound connection, got {other:?}"),
    }
    match next_event(&mut events).await {
        AdapterEvent::PrincipalAuthenticated {
            connection_id: event_connection,
            principal,
            ..
        } => {
            assert_eq!(event_connection, connection_id);
            assert_eq!(principal.issuer.as_deref(), Some("issuer-a"));
            assert_eq!(principal.tenant.as_deref(), Some("tenant-a"));
        }
        other => panic!("expected complete principal event, got {other:?}"),
    }

    let inbound_context =
        ConnectionAdapter::take_inbound_context(server.adapter().as_ref(), &connection_id)
            .expect("WHIP routing context");
    assert_eq!(
        inbound_context
            .routing_hint()
            .expect("WHIP tag")
            .expose_secret(),
        "ownership"
    );
    assert_eq!(inbound_context.transport(), rvoip_core::Transport::WebRtc);
    assert!(
        ConnectionAdapter::take_inbound_context(server.adapter().as_ref(), &connection_id)
            .is_none()
    );

    let retained = server
        .adapter()
        .authenticated_principal(&connection_id)
        .expect("route")
        .expect("complete principal");
    assert_eq!(retained.subject, "shared-subject");
    assert_eq!(retained.issuer.as_deref(), Some("issuer-a"));
    assert_eq!(retained.tenant.as_deref(), Some("tenant-a"));

    let fragment = "a=mid:0\r\na=candidate:1 1 udp 2130706431 127.0.0.1 50001 typ host\r\n";
    let wrong_issuer = http()
        .patch(&route_url)
        .header("authorization", "Bearer other-issuer")
        .header("content-type", "application/trickle-ice-sdpfrag")
        .body(fragment)
        .send()
        .await
        .expect("non-owner PATCH");
    assert_eq!(wrong_issuer.status(), reqwest::StatusCode::FORBIDDEN);

    let wrong_tenant = http()
        .delete(&route_url)
        .header("authorization", "Bearer other-tenant")
        .send()
        .await
        .expect("non-owner DELETE");
    assert_eq!(wrong_tenant.status(), reqwest::StatusCode::FORBIDDEN);
    assert!(server.adapter().routes().contains_key(&connection_id));

    let owner_update = http()
        .patch(&route_url)
        .header("authorization", "Bearer owner")
        .header("content-type", "application/trickle-ice-sdpfrag")
        .body(fragment)
        .send()
        .await
        .expect("owner PATCH");
    assert_eq!(owner_update.status(), reqwest::StatusCode::NO_CONTENT);

    let owner_delete = http()
        .delete(&route_url)
        .header("authorization", "Bearer owner")
        .send()
        .await
        .expect("owner DELETE");
    assert_eq!(owner_delete.status(), reqwest::StatusCode::OK);
    assert!(!server.adapter().routes().contains_key(&connection_id));
    assert!(server
        .adapter()
        .authenticated_principal(&connection_id)
        .is_err());

    // WHEP uses an outbound/originate route, but it is bound to the same
    // adapter-owned principal boundary before the Location id is exposed.
    let whep = http()
        .post(format!("http://{address}/whep/ownership"))
        .header("authorization", "Bearer owner")
        .send()
        .await
        .expect("WHEP POST");
    assert_eq!(whep.status(), reqwest::StatusCode::CREATED);
    let whep_location = whep
        .headers()
        .get("location")
        .expect("WHEP location")
        .to_str()
        .expect("WHEP location text")
        .to_owned();
    let whep_url = format!("http://{address}{whep_location}");
    let whep_id = ConnectionId::from_string(
        whep_location
            .rsplit('/')
            .next()
            .expect("WHEP connection id")
            .to_owned(),
    );
    match next_event(&mut events).await {
        AdapterEvent::Ended {
            connection_id: event_connection,
            ..
        } => assert_eq!(event_connection, connection_id),
        other => panic!("expected WHIP ended event, got {other:?}"),
    }
    match next_event(&mut events).await {
        AdapterEvent::PrincipalAuthenticated {
            connection_id: event_connection,
            principal,
            ..
        } => {
            assert_eq!(event_connection, whep_id);
            assert_eq!(principal.issuer.as_deref(), Some("issuer-a"));
            assert_eq!(principal.tenant.as_deref(), Some("tenant-a"));
        }
        other => panic!("expected WHEP principal event, got {other:?}"),
    }
    let whep_principal = server
        .adapter()
        .authenticated_principal(&whep_id)
        .expect("WHEP route")
        .expect("WHEP principal");
    assert_eq!(whep_principal.ownership_key(), retained.ownership_key());

    let whep_non_owner_update = http()
        .patch(&whep_url)
        .header("authorization", "Bearer other-issuer")
        .header("content-type", "application/sdp")
        .body("v=0\r\n")
        .send()
        .await
        .expect("non-owner WHEP PATCH");
    assert_eq!(
        whep_non_owner_update.status(),
        reqwest::StatusCode::FORBIDDEN
    );
    let whep_non_owner_delete = http()
        .delete(&whep_url)
        .header("authorization", "Bearer other-tenant")
        .send()
        .await
        .expect("non-owner WHEP DELETE");
    assert_eq!(
        whep_non_owner_delete.status(),
        reqwest::StatusCode::FORBIDDEN
    );
    let whep_owner_delete = http()
        .delete(&whep_url)
        .header("authorization", "Bearer owner")
        .send()
        .await
        .expect("owner WHEP DELETE");
    assert_eq!(whep_owner_delete.status(), reqwest::StatusCode::OK);
    assert!(!server.adapter().routes().contains_key(&whep_id));

    server.shutdown().await;
}

#[tokio::test]
async fn websocket_cannot_mutate_whip_route_owned_by_another_principal() {
    let server = start_server().await;
    let offer = fresh_offer().await;
    let (_route_url, connection_id) = create_owned_whip_route(&server, &offer).await;
    let ws_address = server.ws_addr().expect("WS address");

    let (mut attacker, _) =
        tokio_tungstenite::connect_async(format!("ws://{ws_address}/?access_token=other-issuer"))
            .await
            .expect("authenticated attacker upgrade");
    attacker
        .send(Message::Text(
            serde_json::json!({
                "type": "bye",
                "connection_id": connection_id.to_string(),
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("attacker mutation");
    let _ = tokio::time::timeout(Duration::from_secs(2), attacker.next()).await;
    assert!(
        server.adapter().routes().contains_key(&connection_id),
        "cross-protocol non-owner mutation removed the WHIP route"
    );

    let (mut owner, _) =
        tokio_tungstenite::connect_async(format!("ws://{ws_address}/?access_token=owner"))
            .await
            .expect("owner upgrade");
    owner
        .send(Message::Text(
            serde_json::json!({
                "type": "bye",
                "connection_id": connection_id.to_string(),
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("owner mutation");
    for _ in 0..100 {
        if !server.adapter().routes().contains_key(&connection_id) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        !server.adapter().routes().contains_key(&connection_id),
        "same owner should be authorized across signaling protocols"
    );

    server.shutdown().await;
}

#[tokio::test]
async fn websocket_session_hint_becomes_single_take_inbound_context() {
    let server = start_server().await;
    let mut events = server
        .adapter()
        .try_subscribe_events()
        .expect("adapter events");
    let offer = fresh_offer().await;
    let ws_address = server.ws_addr().expect("WS address");
    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{ws_address}/?access_token=owner"))
            .await
            .expect("authenticated websocket upgrade");

    socket
        .send(Message::Text(
            serde_json::json!({
                "type": "offer",
                "sdp": offer,
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send offer");
    let answer = tokio::time::timeout(Duration::from_secs(10), socket.next())
        .await
        .expect("answer timeout")
        .expect("socket open")
        .expect("answer frame");
    assert!(answer.is_text());

    let connection_id = match next_event(&mut events).await {
        AdapterEvent::InboundConnection { connection } => connection.id,
        other => panic!("expected inbound connection, got {other:?}"),
    };
    match next_event(&mut events).await {
        AdapterEvent::PrincipalAuthenticated {
            connection_id: authenticated_id,
            ..
        } => assert_eq!(authenticated_id, connection_id),
        other => panic!("expected principal event, got {other:?}"),
    }

    let context =
        ConnectionAdapter::take_inbound_context(server.adapter().as_ref(), &connection_id)
            .expect("WebSocket context");
    assert_eq!(
        context
            .routing_hint()
            .expect("session hint")
            .expose_secret(),
        "ws-attachment"
    );
    assert!(
        ConnectionAdapter::take_inbound_context(server.adapter().as_ref(), &connection_id)
            .is_none()
    );

    server.shutdown().await;
}
