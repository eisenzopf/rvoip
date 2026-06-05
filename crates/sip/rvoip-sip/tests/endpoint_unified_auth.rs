//! Endpoint UAC to UnifiedCoordinator UAS auth parity tests.
//!
//! These cover real local SIP INVITE challenge/retry flows through the public
//! Endpoint API on the UAC side and `IncomingCall::authenticate_with` on the
//! UnifiedCoordinator UAS side.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serial_test::serial;
use tempfile::TempDir;
use users_core::{
    init, CreateSipDigestCredentialRequest, CreateUserRequest, SipDigestAlgorithmFamily,
    UsersConfig, UsersCoreAuthProvider,
};

use rvoip_core_traits::identity::IdentityAssurance;
use rvoip_core_traits::ids::IdentityId;
use rvoip_sip::api::handlers::AutoAnswerHandler;
use rvoip_sip::api::stream_peer::EventReceiver;
use rvoip_sip::api::AuthScheme;
use rvoip_sip::{
    AkaClientConfig, AkaClientProvider, AkaVectorProvider, AuthIdentity, BearerAuthError,
    BearerValidator, CallId, CallbackPeer, Config, DigestAlgorithm, DigestAuth,
    DigestAuthenticator, Endpoint, EndpointProfile, Event, SessionError, SipAuthChallenge,
    SipAuthDecision, SipAuthScheme, SipAuthService, SipAuthSource, SipClientAuth, StreamPeer,
    UnifiedCoordinator,
};
use rvoip_sip_core::types::headers::HeaderAccess;
use rvoip_sip_core::types::HeaderName;

const BEARER_UAS_PORT: u16 = 36120;
const BEARER_UAC_PORT: u16 = 36121;
const DIGEST_UAS_PORT: u16 = 36122;
const DIGEST_UAC_PORT: u16 = 36123;
const BEARER_TOKEN: &str = "local-dev-token";
const DIGEST_REALM: &str = "pbx.example.com";
const DIGEST_USERNAME: &str = "1001";
const DIGEST_PASSWORD: &str = "sip-secret";
const STREAM_UAS_PORT: u16 = 36140;
const STREAM_UAC_PORT: u16 = 36141;
const CALLBACK_UAS_PORT: u16 = 36142;
const CALLBACK_UAC_PORT: u16 = 36143;
const STREAM_DIGEST_UAS_PORT: u16 = 36144;
const STREAM_DIGEST_UAC_PORT: u16 = 36145;
const CALLBACK_DIGEST_UAS_PORT: u16 = 36146;
const CALLBACK_DIGEST_UAC_PORT: u16 = 36147;
const ENDPOINT_BASIC_UAS_PORT: u16 = 36148;
const ENDPOINT_BASIC_UAC_PORT: u16 = 36149;
const STREAM_BASIC_UAS_PORT: u16 = 36150;
const STREAM_BASIC_UAC_PORT: u16 = 36151;
const CALLBACK_BASIC_UAS_PORT: u16 = 36152;
const CALLBACK_BASIC_UAC_PORT: u16 = 36153;
const ENDPOINT_AKA_UAS_PORT: u16 = 36154;
const ENDPOINT_AKA_UAC_PORT: u16 = 36155;
const STREAM_DIGEST_PROXY_UAS_PORT: u16 = 36156;
const STREAM_DIGEST_PROXY_UAC_PORT: u16 = 36157;
const CALLBACK_DIGEST_PROXY_UAS_PORT: u16 = 36158;
const CALLBACK_DIGEST_PROXY_UAC_PORT: u16 = 36159;
const STREAM_AKA_UAS_PORT: u16 = 36160;
const STREAM_AKA_UAC_PORT: u16 = 36161;
const CALLBACK_AKA_UAS_PORT: u16 = 36162;
const CALLBACK_AKA_UAC_PORT: u16 = 36163;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn endpoint_uac_retries_bearer_invite_against_unified_uas() -> anyhow::Result<()> {
    let uas_auth = SipAuthService::new()
        .with_bearer_validator("local-dev", Arc::new(StaticBearerValidator))
        .with_bearer_scope("sip.invite")
        .allow_bearer_over_cleartext(true);

    let identity = run_endpoint_invite_auth_flow(
        BEARER_UAS_PORT,
        BEARER_UAC_PORT,
        uas_auth,
        SipAuthScheme::Bearer,
        SipAuthSource::Origin,
        SipClientAuth::bearer_token(BEARER_TOKEN).allow_bearer_over_cleartext(true),
    )
    .await?;

    assert_eq!(identity.scheme, SipAuthScheme::Bearer);
    assert_eq!(identity.source, SipAuthSource::Origin);
    assert_eq!(identity.subject.as_deref(), Some("user_alice"));
    assert!(identity.scopes.iter().any(|scope| scope == "sip.invite"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn endpoint_uac_retries_digest_invite_against_unified_uas() -> anyhow::Result<()> {
    let (_temp_dir, users) = seed_users_core().await?;
    let provider = UsersCoreAuthProvider::shared(Arc::new(users));
    let uas_auth = SipAuthService::new()
        .with_digest_provider(DIGEST_REALM, provider)
        .with_digest_provider_algorithm(DigestAlgorithm::SHA256);

    let identity = run_endpoint_invite_auth_flow(
        DIGEST_UAS_PORT,
        DIGEST_UAC_PORT,
        uas_auth,
        SipAuthScheme::Digest,
        SipAuthSource::Origin,
        SipClientAuth::digest(DIGEST_USERNAME, DIGEST_PASSWORD),
    )
    .await?;

    assert_eq!(identity.scheme, SipAuthScheme::Digest);
    assert_eq!(identity.source, SipAuthSource::Origin);
    assert_eq!(identity.username.as_deref(), Some(DIGEST_USERNAME));
    assert_eq!(identity.realm.as_deref(), Some(DIGEST_REALM));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn endpoint_uac_retries_digest_proxy_challenge_with_proxy_authorization() -> anyhow::Result<()>
{
    let (_temp_dir, users) = seed_users_core().await?;
    let provider = UsersCoreAuthProvider::shared(Arc::new(users));
    let uas_auth = SipAuthService::new()
        .with_digest_provider(DIGEST_REALM, provider)
        .with_digest_provider_algorithm(DigestAlgorithm::SHA256);

    let identity = run_endpoint_invite_auth_flow(
        DIGEST_UAS_PORT + 2,
        DIGEST_UAC_PORT + 2,
        uas_auth,
        SipAuthScheme::Digest,
        SipAuthSource::Proxy,
        SipClientAuth::digest(DIGEST_USERNAME, DIGEST_PASSWORD),
    )
    .await?;

    assert_eq!(identity.scheme, SipAuthScheme::Digest);
    assert_eq!(identity.source, SipAuthSource::Proxy);
    assert_eq!(identity.username.as_deref(), Some(DIGEST_USERNAME));
    Ok(())
}

#[tokio::test]
async fn users_core_digest_rotation_and_deletion_apply_to_sip_auth_service() -> anyhow::Result<()> {
    let (_temp_dir, users) = seed_users_core().await?;
    let users = Arc::new(users);
    let user = users
        .user_store()
        .get_user_by_username("alice")
        .await?
        .ok_or_else(|| anyhow::anyhow!("seeded users-core user was not found"))?;
    let provider = UsersCoreAuthProvider::shared(users.clone());
    let service = SipAuthService::new()
        .with_digest_provider(DIGEST_REALM, provider)
        .with_digest_provider_algorithm(DigestAlgorithm::SHA256);
    let method = "INVITE";
    let request_uri = "sip:bob@example.com";

    let initial = digest_authorization_for(&service, DIGEST_PASSWORD, method, request_uri).await?;
    let initial_decision = service
        .authenticate_authorization(
            Some(&initial),
            method,
            request_uri,
            None,
            SipAuthSource::Origin,
            false,
        )
        .await?;
    assert_digest_authorized(initial_decision);

    users
        .rotate_sip_digest_credential(
            user.id.clone(),
            DIGEST_USERNAME,
            DIGEST_REALM,
            SipDigestAlgorithmFamily::Sha256,
            "sip-secret-two",
        )
        .await?;

    let old_password =
        digest_authorization_for(&service, DIGEST_PASSWORD, method, request_uri).await?;
    let old_password_decision = service
        .authenticate_authorization(
            Some(&old_password),
            method,
            request_uri,
            None,
            SipAuthSource::Origin,
            false,
        )
        .await?;
    assert!(
        matches!(old_password_decision, SipAuthDecision::Rejected { .. }),
        "rotated SIP Digest credentials must reject the old password"
    );

    let rotated = digest_authorization_for(&service, "sip-secret-two", method, request_uri).await?;
    let rotated_decision = service
        .authenticate_authorization(
            Some(&rotated),
            method,
            request_uri,
            None,
            SipAuthSource::Origin,
            false,
        )
        .await?;
    assert_digest_authorized(rotated_decision);

    users
        .delete_sip_digest_credential(
            DIGEST_USERNAME,
            DIGEST_REALM,
            SipDigestAlgorithmFamily::Sha256,
        )
        .await?;
    let deleted = digest_authorization_for(&service, "sip-secret-two", method, request_uri).await?;
    let deleted_decision = service
        .authenticate_authorization(
            Some(&deleted),
            method,
            request_uri,
            None,
            SipAuthSource::Origin,
            false,
        )
        .await?;
    assert!(
        matches!(deleted_decision, SipAuthDecision::Rejected { .. }),
        "deleted SIP Digest credentials must be rejected by SipAuthService"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn endpoint_uac_retries_basic_invite_against_unified_uas() -> anyhow::Result<()> {
    let (_temp_dir, users) = seed_users_core().await?;
    let provider = UsersCoreAuthProvider::shared(Arc::new(users));
    let uas_auth = SipAuthService::new()
        .with_basic_verifier("users-core", provider)
        .allow_basic_over_cleartext(true);

    let identity = run_endpoint_invite_auth_flow(
        ENDPOINT_BASIC_UAS_PORT,
        ENDPOINT_BASIC_UAC_PORT,
        uas_auth,
        SipAuthScheme::Basic,
        SipAuthSource::Origin,
        SipClientAuth::basic("alice", "SecurePass2024").allow_basic_over_cleartext(true),
    )
    .await?;

    assert_eq!(identity.scheme, SipAuthScheme::Basic);
    assert_eq!(identity.source, SipAuthSource::Origin);
    assert_eq!(identity.username.as_deref(), Some("alice"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn endpoint_uac_retries_aka_provider_shape_against_unified_uas() -> anyhow::Result<()> {
    let aka_provider = Arc::new(StaticAkaProvider);
    let uas_auth = SipAuthService::new().with_aka_provider(aka_provider.clone());

    let identity = run_endpoint_invite_auth_flow(
        ENDPOINT_AKA_UAS_PORT,
        ENDPOINT_AKA_UAC_PORT,
        uas_auth,
        SipAuthScheme::Aka,
        SipAuthSource::Origin,
        SipClientAuth::aka(AkaClientConfig::new(aka_provider)),
    )
    .await?;

    assert_eq!(identity.scheme, SipAuthScheme::Aka);
    assert_eq!(identity.source, SipAuthSource::Origin);
    assert_eq!(identity.username.as_deref(), Some("aka-user"));
    assert_eq!(identity.realm.as_deref(), Some("ims.example.com"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn stream_peer_retries_bearer_invite_against_unified_uas() -> anyhow::Result<()> {
    let (uas, uas_task) = spawn_auth_challenging_uas(
        STREAM_UAS_PORT,
        bearer_uas_auth(),
        SipAuthScheme::Bearer,
        SipAuthSource::Origin,
    )
    .await?;

    let mut peer = StreamPeer::builder()
        .config(Config::local("stream-auth-uac", STREAM_UAC_PORT).with_signaling_only_media(9))
        .with_auth(SipClientAuth::bearer_token(BEARER_TOKEN).allow_bearer_over_cleartext(true))
        .build()
        .await?;

    let call_id = peer
        .invite(format!("sip:bob@127.0.0.1:{STREAM_UAS_PORT}"))
        .send()
        .await?;
    let handle = peer.wait_for_answered(&call_id).await?;
    handle.hangup_and_wait(Some(Duration::from_secs(5))).await?;
    peer.shutdown().await?;

    let identity = uas_task.await.map_err(|err| anyhow::anyhow!(err))??;
    uas.shutdown_gracefully(Some(Duration::from_secs(2)))
        .await?;

    assert_eq!(identity.scheme, SipAuthScheme::Bearer);
    assert_eq!(identity.source, SipAuthSource::Origin);
    assert_eq!(identity.subject.as_deref(), Some("user_alice"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn stream_peer_retries_digest_invite_against_unified_uas() -> anyhow::Result<()> {
    let (_temp_dir, users) = seed_users_core().await?;
    let provider = UsersCoreAuthProvider::shared(Arc::new(users));
    let uas_auth = SipAuthService::new()
        .with_digest_provider(DIGEST_REALM, provider)
        .with_digest_provider_algorithm(DigestAlgorithm::SHA256);
    let (uas, uas_task) = spawn_auth_challenging_uas(
        STREAM_DIGEST_UAS_PORT,
        uas_auth,
        SipAuthScheme::Digest,
        SipAuthSource::Origin,
    )
    .await?;

    let mut peer = StreamPeer::builder()
        .config(
            Config::local("stream-digest-uac", STREAM_DIGEST_UAC_PORT).with_signaling_only_media(9),
        )
        .with_auth(SipClientAuth::digest(DIGEST_USERNAME, DIGEST_PASSWORD))
        .build()
        .await?;

    let call_id = peer
        .invite(format!("sip:bob@127.0.0.1:{STREAM_DIGEST_UAS_PORT}"))
        .send()
        .await?;
    let handle = peer.wait_for_answered(&call_id).await?;
    handle.hangup_and_wait(Some(Duration::from_secs(5))).await?;
    peer.shutdown().await?;

    let identity = uas_task.await.map_err(|err| anyhow::anyhow!(err))??;
    uas.shutdown_gracefully(Some(Duration::from_secs(2)))
        .await?;

    assert_eq!(identity.scheme, SipAuthScheme::Digest);
    assert_eq!(identity.source, SipAuthSource::Origin);
    assert_eq!(identity.username.as_deref(), Some(DIGEST_USERNAME));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn stream_peer_retries_digest_proxy_challenge_with_proxy_authorization() -> anyhow::Result<()>
{
    let (_temp_dir, users) = seed_users_core().await?;
    let provider = UsersCoreAuthProvider::shared(Arc::new(users));
    let uas_auth = SipAuthService::new()
        .with_digest_provider(DIGEST_REALM, provider)
        .with_digest_provider_algorithm(DigestAlgorithm::SHA256);
    let (uas, uas_task) = spawn_auth_challenging_uas(
        STREAM_DIGEST_PROXY_UAS_PORT,
        uas_auth,
        SipAuthScheme::Digest,
        SipAuthSource::Proxy,
    )
    .await?;

    let mut peer = StreamPeer::builder()
        .config(
            Config::local("stream-digest-proxy-uac", STREAM_DIGEST_PROXY_UAC_PORT)
                .with_signaling_only_media(9),
        )
        .with_auth(SipClientAuth::digest(DIGEST_USERNAME, DIGEST_PASSWORD))
        .build()
        .await?;

    let call_id = peer
        .invite(format!("sip:bob@127.0.0.1:{STREAM_DIGEST_PROXY_UAS_PORT}"))
        .send()
        .await?;
    let handle = peer.wait_for_answered(&call_id).await?;
    handle.hangup_and_wait(Some(Duration::from_secs(5))).await?;
    peer.shutdown().await?;

    let identity = uas_task.await.map_err(|err| anyhow::anyhow!(err))??;
    uas.shutdown_gracefully(Some(Duration::from_secs(2)))
        .await?;

    assert_eq!(identity.scheme, SipAuthScheme::Digest);
    assert_eq!(identity.source, SipAuthSource::Proxy);
    assert_eq!(identity.username.as_deref(), Some(DIGEST_USERNAME));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn stream_peer_retries_basic_invite_against_unified_uas() -> anyhow::Result<()> {
    let (_temp_dir, users) = seed_users_core().await?;
    let provider = UsersCoreAuthProvider::shared(Arc::new(users));
    let uas_auth = SipAuthService::new()
        .with_basic_verifier("users-core", provider)
        .allow_basic_over_cleartext(true);
    let (uas, uas_task) = spawn_auth_challenging_uas(
        STREAM_BASIC_UAS_PORT,
        uas_auth,
        SipAuthScheme::Basic,
        SipAuthSource::Origin,
    )
    .await?;

    let mut peer = StreamPeer::builder()
        .config(
            Config::local("stream-basic-uac", STREAM_BASIC_UAC_PORT).with_signaling_only_media(9),
        )
        .with_auth(SipClientAuth::basic("alice", "SecurePass2024").allow_basic_over_cleartext(true))
        .build()
        .await?;

    let call_id = peer
        .invite(format!("sip:bob@127.0.0.1:{STREAM_BASIC_UAS_PORT}"))
        .send()
        .await?;
    let handle = peer.wait_for_answered(&call_id).await?;
    handle.hangup_and_wait(Some(Duration::from_secs(5))).await?;
    peer.shutdown().await?;

    let identity = uas_task.await.map_err(|err| anyhow::anyhow!(err))??;
    uas.shutdown_gracefully(Some(Duration::from_secs(2)))
        .await?;

    assert_eq!(identity.scheme, SipAuthScheme::Basic);
    assert_eq!(identity.source, SipAuthSource::Origin);
    assert_eq!(identity.username.as_deref(), Some("alice"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn stream_peer_retries_aka_provider_shape_against_unified_uas() -> anyhow::Result<()> {
    let aka_provider = Arc::new(StaticAkaProvider);
    let uas_auth = SipAuthService::new().with_aka_provider(aka_provider.clone());
    let (uas, uas_task) = spawn_auth_challenging_uas(
        STREAM_AKA_UAS_PORT,
        uas_auth,
        SipAuthScheme::Aka,
        SipAuthSource::Origin,
    )
    .await?;

    let mut peer = StreamPeer::builder()
        .config(Config::local("stream-aka-uac", STREAM_AKA_UAC_PORT).with_signaling_only_media(9))
        .with_auth(SipClientAuth::aka(AkaClientConfig::new(aka_provider)))
        .build()
        .await?;

    let call_id = peer
        .invite(format!("sip:bob@127.0.0.1:{STREAM_AKA_UAS_PORT}"))
        .send()
        .await?;
    let handle = peer.wait_for_answered(&call_id).await?;
    handle.hangup_and_wait(Some(Duration::from_secs(5))).await?;
    peer.shutdown().await?;

    let identity = uas_task.await.map_err(|err| anyhow::anyhow!(err))??;
    uas.shutdown_gracefully(Some(Duration::from_secs(2)))
        .await?;

    assert_eq!(identity.scheme, SipAuthScheme::Aka);
    assert_eq!(identity.source, SipAuthSource::Origin);
    assert_eq!(identity.username.as_deref(), Some("aka-user"));
    assert_eq!(identity.realm.as_deref(), Some("ims.example.com"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn callback_peer_retries_bearer_invite_against_unified_uas() -> anyhow::Result<()> {
    let (uas, uas_task) = spawn_auth_challenging_uas(
        CALLBACK_UAS_PORT,
        bearer_uas_auth(),
        SipAuthScheme::Bearer,
        SipAuthSource::Origin,
    )
    .await?;

    let mut config =
        Config::local("callback-auth-uac", CALLBACK_UAC_PORT).with_signaling_only_media(9);
    config.auth = Some(SipClientAuth::bearer_token(BEARER_TOKEN).allow_bearer_over_cleartext(true));
    let peer = CallbackPeer::new(AutoAnswerHandler, config).await?;
    let control = peer.control();
    let coord = control.coordinator().clone();
    let peer_task = tokio::spawn(async move { peer.run().await });

    let call_id = control
        .invite(format!("sip:bob@127.0.0.1:{CALLBACK_UAS_PORT}"))
        .send()
        .await?;
    let mut events = coord.events_for_session(&call_id).await?;
    wait_for_call_answered(&mut events, &call_id).await?;
    coord.hangup(&call_id).await?;
    control.shutdown();

    let identity = uas_task.await.map_err(|err| anyhow::anyhow!(err))??;
    peer_task.await.map_err(|err| anyhow::anyhow!(err))??;
    uas.shutdown_gracefully(Some(Duration::from_secs(2)))
        .await?;

    assert_eq!(identity.scheme, SipAuthScheme::Bearer);
    assert_eq!(identity.source, SipAuthSource::Origin);
    assert_eq!(identity.subject.as_deref(), Some("user_alice"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn callback_peer_retries_basic_invite_against_unified_uas() -> anyhow::Result<()> {
    let (_temp_dir, users) = seed_users_core().await?;
    let provider = UsersCoreAuthProvider::shared(Arc::new(users));
    let uas_auth = SipAuthService::new()
        .with_basic_verifier("users-core", provider)
        .allow_basic_over_cleartext(true);
    let (uas, uas_task) = spawn_auth_challenging_uas(
        CALLBACK_BASIC_UAS_PORT,
        uas_auth,
        SipAuthScheme::Basic,
        SipAuthSource::Origin,
    )
    .await?;

    let mut config =
        Config::local("callback-basic-uac", CALLBACK_BASIC_UAC_PORT).with_signaling_only_media(9);
    config.auth =
        Some(SipClientAuth::basic("alice", "SecurePass2024").allow_basic_over_cleartext(true));
    let peer = CallbackPeer::new(AutoAnswerHandler, config).await?;
    let control = peer.control();
    let coord = control.coordinator().clone();
    let peer_task = tokio::spawn(async move { peer.run().await });

    let call_id = control
        .invite(format!("sip:bob@127.0.0.1:{CALLBACK_BASIC_UAS_PORT}"))
        .send()
        .await?;
    let mut events = coord.events_for_session(&call_id).await?;
    wait_for_call_answered(&mut events, &call_id).await?;
    coord.hangup(&call_id).await?;
    control.shutdown();

    let identity = uas_task.await.map_err(|err| anyhow::anyhow!(err))??;
    peer_task.await.map_err(|err| anyhow::anyhow!(err))??;
    uas.shutdown_gracefully(Some(Duration::from_secs(2)))
        .await?;

    assert_eq!(identity.scheme, SipAuthScheme::Basic);
    assert_eq!(identity.source, SipAuthSource::Origin);
    assert_eq!(identity.username.as_deref(), Some("alice"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn callback_peer_retries_digest_invite_against_unified_uas() -> anyhow::Result<()> {
    let (_temp_dir, users) = seed_users_core().await?;
    let provider = UsersCoreAuthProvider::shared(Arc::new(users));
    let uas_auth = SipAuthService::new()
        .with_digest_provider(DIGEST_REALM, provider)
        .with_digest_provider_algorithm(DigestAlgorithm::SHA256);
    let (uas, uas_task) = spawn_auth_challenging_uas(
        CALLBACK_DIGEST_UAS_PORT,
        uas_auth,
        SipAuthScheme::Digest,
        SipAuthSource::Origin,
    )
    .await?;

    let mut config =
        Config::local("callback-digest-uac", CALLBACK_DIGEST_UAC_PORT).with_signaling_only_media(9);
    config.auth = Some(SipClientAuth::digest(DIGEST_USERNAME, DIGEST_PASSWORD));
    let peer = CallbackPeer::new(AutoAnswerHandler, config).await?;
    let control = peer.control();
    let coord = control.coordinator().clone();
    let peer_task = tokio::spawn(async move { peer.run().await });

    let call_id = control
        .invite(format!("sip:bob@127.0.0.1:{CALLBACK_DIGEST_UAS_PORT}"))
        .send()
        .await?;
    let mut events = coord.events_for_session(&call_id).await?;
    wait_for_call_answered(&mut events, &call_id).await?;
    coord.hangup(&call_id).await?;
    control.shutdown();

    let identity = uas_task.await.map_err(|err| anyhow::anyhow!(err))??;
    peer_task.await.map_err(|err| anyhow::anyhow!(err))??;
    uas.shutdown_gracefully(Some(Duration::from_secs(2)))
        .await?;

    assert_eq!(identity.scheme, SipAuthScheme::Digest);
    assert_eq!(identity.source, SipAuthSource::Origin);
    assert_eq!(identity.username.as_deref(), Some(DIGEST_USERNAME));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn callback_peer_retries_digest_proxy_challenge_with_proxy_authorization(
) -> anyhow::Result<()> {
    let (_temp_dir, users) = seed_users_core().await?;
    let provider = UsersCoreAuthProvider::shared(Arc::new(users));
    let uas_auth = SipAuthService::new()
        .with_digest_provider(DIGEST_REALM, provider)
        .with_digest_provider_algorithm(DigestAlgorithm::SHA256);
    let (uas, uas_task) = spawn_auth_challenging_uas(
        CALLBACK_DIGEST_PROXY_UAS_PORT,
        uas_auth,
        SipAuthScheme::Digest,
        SipAuthSource::Proxy,
    )
    .await?;

    let mut config = Config::local("callback-digest-proxy-uac", CALLBACK_DIGEST_PROXY_UAC_PORT)
        .with_signaling_only_media(9);
    config.auth = Some(SipClientAuth::digest(DIGEST_USERNAME, DIGEST_PASSWORD));
    let peer = CallbackPeer::new(AutoAnswerHandler, config).await?;
    let control = peer.control();
    let coord = control.coordinator().clone();
    let peer_task = tokio::spawn(async move { peer.run().await });

    let call_id = control
        .invite(format!(
            "sip:bob@127.0.0.1:{CALLBACK_DIGEST_PROXY_UAS_PORT}"
        ))
        .send()
        .await?;
    let mut events = coord.events_for_session(&call_id).await?;
    wait_for_call_answered(&mut events, &call_id).await?;
    coord.hangup(&call_id).await?;
    control.shutdown();

    let identity = uas_task.await.map_err(|err| anyhow::anyhow!(err))??;
    peer_task.await.map_err(|err| anyhow::anyhow!(err))??;
    uas.shutdown_gracefully(Some(Duration::from_secs(2)))
        .await?;

    assert_eq!(identity.scheme, SipAuthScheme::Digest);
    assert_eq!(identity.source, SipAuthSource::Proxy);
    assert_eq!(identity.username.as_deref(), Some(DIGEST_USERNAME));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn callback_peer_retries_aka_provider_shape_against_unified_uas() -> anyhow::Result<()> {
    let aka_provider = Arc::new(StaticAkaProvider);
    let uas_auth = SipAuthService::new().with_aka_provider(aka_provider.clone());
    let (uas, uas_task) = spawn_auth_challenging_uas(
        CALLBACK_AKA_UAS_PORT,
        uas_auth,
        SipAuthScheme::Aka,
        SipAuthSource::Origin,
    )
    .await?;

    let mut config =
        Config::local("callback-aka-uac", CALLBACK_AKA_UAC_PORT).with_signaling_only_media(9);
    config.auth = Some(SipClientAuth::aka(AkaClientConfig::new(aka_provider)));
    let peer = CallbackPeer::new(AutoAnswerHandler, config).await?;
    let control = peer.control();
    let coord = control.coordinator().clone();
    let peer_task = tokio::spawn(async move { peer.run().await });

    let call_id = control
        .invite(format!("sip:bob@127.0.0.1:{CALLBACK_AKA_UAS_PORT}"))
        .send()
        .await?;
    let mut events = coord.events_for_session(&call_id).await?;
    wait_for_call_answered(&mut events, &call_id).await?;
    coord.hangup(&call_id).await?;
    control.shutdown();

    let identity = uas_task.await.map_err(|err| anyhow::anyhow!(err))??;
    peer_task.await.map_err(|err| anyhow::anyhow!(err))??;
    uas.shutdown_gracefully(Some(Duration::from_secs(2)))
        .await?;

    assert_eq!(identity.scheme, SipAuthScheme::Aka);
    assert_eq!(identity.source, SipAuthSource::Origin);
    assert_eq!(identity.username.as_deref(), Some("aka-user"));
    assert_eq!(identity.realm.as_deref(), Some("ims.example.com"));
    Ok(())
}

async fn run_endpoint_invite_auth_flow(
    uas_port: u16,
    uac_port: u16,
    uas_auth: SipAuthService,
    expected_challenge_scheme: SipAuthScheme,
    challenge_source: SipAuthSource,
    uac_auth: SipClientAuth,
) -> anyhow::Result<AuthIdentity> {
    let (uas, uas_task) = spawn_auth_challenging_uas(
        uas_port,
        uas_auth,
        expected_challenge_scheme,
        challenge_source,
    )
    .await?;

    tokio::time::sleep(Duration::from_millis(300)).await;

    let endpoint = Endpoint::builder()
        .name("auth-parity-uac")
        .profile(EndpointProfile::Custom(
            Config::local("auth-parity-uac", uac_port).with_signaling_only_media(9),
        ))
        .auth(uac_auth)
        .build()
        .await?;

    let call = endpoint
        .call_and_wait(
            &format!("sip:bob@127.0.0.1:{uas_port}"),
            Some(Duration::from_secs(10)),
        )
        .await?;
    call.hangup_and_wait(Some(Duration::from_secs(5))).await?;
    endpoint.shutdown().await?;

    let identity = uas_task.await.map_err(|err| anyhow::anyhow!(err))??;
    uas.shutdown_gracefully(Some(Duration::from_secs(2)))
        .await?;

    Ok(identity)
}

async fn spawn_auth_challenging_uas(
    uas_port: u16,
    uas_auth: SipAuthService,
    expected_challenge_scheme: SipAuthScheme,
    challenge_source: SipAuthSource,
) -> anyhow::Result<(
    Arc<UnifiedCoordinator>,
    tokio::task::JoinHandle<std::result::Result<AuthIdentity, SessionError>>,
)> {
    let uas = UnifiedCoordinator::new(
        Config::local("auth-parity-uas", uas_port).with_signaling_only_media(9),
    )
    .await?;
    let mut uas_events = uas.events().await?;
    let task = {
        let uas = uas.clone();
        tokio::spawn(async move {
            loop {
                let Some(incoming) = uas.next_incoming_call(&mut uas_events).await? else {
                    return Err(SessionError::Other(
                        "UAS event stream closed before call completed".to_string(),
                    ));
                };

                match incoming.authenticate_with(&uas_auth).await? {
                    SipAuthDecision::Authorized(identity) => {
                        assert_auth_retry_header(
                            &incoming,
                            expected_challenge_scheme.clone(),
                            identity.source,
                        )
                        .map_err(|err| SessionError::Other(err.to_string()))?;
                        let accepted_identity = identity.clone();
                        let call = incoming.accept().await?;
                        call.wait_for_end(Some(Duration::from_secs(10))).await?;
                        return Ok::<_, SessionError>(accepted_identity);
                    }
                    SipAuthDecision::Rejected { challenges } => {
                        assert_no_auth_headers(&incoming)
                            .map_err(|err| SessionError::Other(err.to_string()))?;
                        let challenge = challenges
                            .into_iter()
                            .find(|challenge| challenge.scheme == expected_challenge_scheme)
                            .ok_or_else(|| {
                                SessionError::AuthError(format!(
                                    "{expected_challenge_scheme:?} challenge was not generated"
                                ))
                            })?;
                        incoming
                            .challenge_builder(to_auth_scheme(&challenge.scheme))
                            .with_auth_challenge(&challenge)
                            .as_proxy_challenge(challenge_source == SipAuthSource::Proxy)
                            .send()
                            .await?;
                    }
                }
            }
        })
    };

    Ok((uas, task))
}

fn assert_no_auth_headers(incoming: &rvoip_sip::IncomingCall) -> anyhow::Result<()> {
    let request = incoming
        .raw_request()
        .ok_or_else(|| anyhow::anyhow!("missing raw INVITE"))?;
    assert!(
        request
            .raw_header_value(&HeaderName::Authorization)
            .is_none(),
        "initial INVITE must not carry Authorization"
    );
    assert!(
        request
            .raw_header_value(&HeaderName::ProxyAuthorization)
            .is_none(),
        "initial INVITE must not carry Proxy-Authorization"
    );
    Ok(())
}

fn assert_auth_retry_header(
    incoming: &rvoip_sip::IncomingCall,
    scheme: SipAuthScheme,
    source: SipAuthSource,
) -> anyhow::Result<()> {
    let request = incoming
        .raw_request()
        .ok_or_else(|| anyhow::anyhow!("missing raw INVITE"))?;
    let (expected, forbidden) = match source {
        SipAuthSource::Origin => (HeaderName::Authorization, HeaderName::ProxyAuthorization),
        SipAuthSource::Proxy => (HeaderName::ProxyAuthorization, HeaderName::Authorization),
    };
    let value = request.raw_header_value(&expected).ok_or_else(|| {
        anyhow::anyhow!("authenticated retry missing expected {expected:?} header")
    })?;
    assert!(
        request.raw_header_value(&forbidden).is_none(),
        "authenticated retry used both origin and proxy auth headers"
    );
    match scheme {
        SipAuthScheme::Digest => {
            assert!(
                value.starts_with("Digest ") && value.contains("response="),
                "Digest retry must carry a full digest response, got {value:?}"
            );
        }
        SipAuthScheme::Bearer => {
            assert!(
                value.starts_with("Bearer ") && value.len() > "Bearer ".len(),
                "Bearer retry must carry a token, got {value:?}"
            );
        }
        SipAuthScheme::Basic => {
            assert!(
                value.starts_with("Basic ") && value.len() > "Basic ".len(),
                "Basic retry must carry credentials, got {value:?}"
            );
        }
        SipAuthScheme::Aka => {
            let upper = value.to_ascii_uppercase();
            assert!(
                value.starts_with("Digest ") && upper.contains("ALGORITHM=AKAV1-MD5"),
                "AKA retry must carry a Digest-family AKA response, got {value:?}"
            );
        }
        other => panic!("unexpected scheme in Endpoint/Unified auth test: {other:?}"),
    }
    Ok(())
}

async fn digest_authorization_for(
    service: &SipAuthService,
    password: &str,
    method: &str,
    request_uri: &str,
) -> anyhow::Result<String> {
    let challenge = service
        .challenges_async(SipAuthSource::Origin)
        .await?
        .into_iter()
        .find(|challenge| challenge.scheme == SipAuthScheme::Digest)
        .ok_or_else(|| anyhow::anyhow!("Digest challenge was not generated"))?;
    let digest_challenge = DigestAuthenticator::parse_challenge(&challenge.value)?;
    let computed = DigestAuth::compute_response_with_state(
        DIGEST_USERNAME,
        password,
        &digest_challenge,
        method,
        request_uri,
        1,
        None,
    )?;
    Ok(DigestAuth::format_authorization_with_state(
        DIGEST_USERNAME,
        &digest_challenge,
        request_uri,
        &computed,
    ))
}

fn assert_digest_authorized(decision: SipAuthDecision) {
    match decision {
        SipAuthDecision::Authorized(identity) => {
            assert_eq!(identity.scheme, SipAuthScheme::Digest);
            assert_eq!(identity.username.as_deref(), Some(DIGEST_USERNAME));
            assert_eq!(identity.realm.as_deref(), Some(DIGEST_REALM));
            assert_eq!(identity.source, SipAuthSource::Origin);
        }
        other => panic!("expected Digest authorization, got {other:?}"),
    }
}

async fn wait_for_call_answered(
    events: &mut EventReceiver,
    call_id: &CallId,
) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match events.next().await {
                Some(Event::CallAnswered {
                    call_id: answered, ..
                }) if &answered == call_id => return Ok(()),
                Some(Event::CallFailed {
                    call_id: failed,
                    status_code,
                    reason,
                }) if &failed == call_id => {
                    return Err(anyhow::anyhow!(
                        "call failed before answer: {status_code} {reason}"
                    ));
                }
                Some(_) => continue,
                None => return Err(anyhow::anyhow!("event stream closed before CallAnswered")),
            }
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("timed out waiting for CallAnswered"))?
}

async fn seed_users_core() -> anyhow::Result<(TempDir, users_core::AuthenticationService)> {
    let temp_dir = TempDir::new()?;
    let db_url = format!(
        "sqlite://{}?mode=rwc",
        temp_dir.path().join("users.db").display()
    );
    let users = init(UsersConfig {
        database_url: db_url,
        ..UsersConfig::default()
    })
    .await?;

    let user = users
        .create_user(CreateUserRequest {
            username: "alice".to_string(),
            password: "SecurePass2024".to_string(),
            email: Some("alice@example.com".to_string()),
            display_name: Some("Alice".to_string()),
            roles: vec!["user".to_string()],
        })
        .await?;

    users
        .create_sip_digest_credential(CreateSipDigestCredentialRequest {
            user_id: user.id,
            sip_username: DIGEST_USERNAME.to_string(),
            realm: DIGEST_REALM.to_string(),
            algorithm: SipDigestAlgorithmFamily::Sha256,
            password: DIGEST_PASSWORD.to_string(),
        })
        .await?;

    Ok((temp_dir, users))
}

struct StaticBearerValidator;

fn bearer_uas_auth() -> SipAuthService {
    SipAuthService::new()
        .with_bearer_validator("local-dev", Arc::new(StaticBearerValidator))
        .with_bearer_scope("sip.invite")
        .allow_bearer_over_cleartext(true)
}

struct StaticAkaProvider;

impl AkaClientProvider for StaticAkaProvider {
    fn authorization(
        &self,
        challenge_header: &str,
        method: &str,
        request_uri: &str,
        nonce_count: u32,
    ) -> rvoip_sip::Result<String> {
        if !challenge_header.to_ascii_uppercase().contains("AKAV1-MD5") {
            return Err(SessionError::AuthError(
                "AKA challenge did not offer AKAv1-MD5".to_string(),
            ));
        }
        Ok(format!(
            r#"Digest username="aka-user", realm="ims.example.com", nonce="aka-nonce", uri="{request_uri}", response="aka-response", algorithm=AKAv1-MD5, qop=auth, nc={nonce_count:08x}, cnonce="aka-cnonce", method="{method}""#
        ))
    }
}

#[async_trait]
impl AkaVectorProvider for StaticAkaProvider {
    async fn validate(
        &self,
        authorization: &str,
        _method: &str,
        _request_uri: &str,
        _body: Option<&[u8]>,
    ) -> rvoip_sip::Result<Option<AuthIdentity>> {
        let upper = authorization.to_ascii_uppercase();
        if authorization.contains(r#"username="aka-user""#)
            && upper.contains("AKAV1-MD5")
            && authorization.contains(r#"response="aka-response""#)
        {
            return Ok(Some(AuthIdentity {
                scheme: SipAuthScheme::Aka,
                username: Some("aka-user".to_string()),
                subject: Some("imsi-001010000000001".to_string()),
                realm: Some("ims.example.com".to_string()),
                scopes: vec!["sip.invite".to_string()],
                source: SipAuthSource::Origin,
            }));
        }
        Ok(None)
    }

    fn challenge(&self, source: SipAuthSource) -> SipAuthChallenge {
        SipAuthChallenge {
            scheme: SipAuthScheme::Aka,
            value: r#"Digest realm="ims.example.com", nonce="aka-nonce", algorithm=AKAv1-MD5, qop="auth""#
                .to_string(),
            source,
        }
    }
}

#[async_trait]
impl BearerValidator for StaticBearerValidator {
    async fn validate(
        &self,
        token: &str,
    ) -> std::result::Result<IdentityAssurance, BearerAuthError> {
        if token != BEARER_TOKEN {
            return Err(BearerAuthError::Invalid("invalid token".to_string()));
        }

        let identity = IdentityId::from_string("user_alice");
        Ok(IdentityAssurance::UserAuthorized {
            identity: identity.clone(),
            user_id: identity,
            scopes: vec!["sip.invite".to_string()],
        })
    }
}

fn to_auth_scheme(scheme: &SipAuthScheme) -> AuthScheme {
    match scheme {
        SipAuthScheme::Digest => AuthScheme::Digest,
        SipAuthScheme::Bearer => AuthScheme::Bearer,
        SipAuthScheme::Basic => AuthScheme::Basic,
        SipAuthScheme::Aka => AuthScheme::Aka,
        SipAuthScheme::Other(_) => AuthScheme::Digest,
        _ => AuthScheme::Digest,
    }
}
