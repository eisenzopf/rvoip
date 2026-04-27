//! Guard Condition Unit Tests
//!
//! Tests all guard variants in `check_guard()` from `state_machine/guards.rs`.
//! Guards are pure functions on SessionState — no adapters or network needed.

use rvoip_session_core::internals::{Guard, NegotiatedConfig, Role, SessionId, SessionState};
use rvoip_session_core::state_machine::guards::check_guard;
use rvoip_session_core::types::CallState;
use std::net::SocketAddr;

fn make_session() -> SessionState {
    SessionState::new(SessionId::new(), Role::UAC)
}

// ── SDP guards ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_has_local_sdp_true() {
    let mut s = make_session();
    s.local_sdp = Some("v=0\r\n".into());
    assert!(check_guard(&Guard::HasLocalSDP, &s).await);
}

#[tokio::test]
async fn test_has_local_sdp_false() {
    let s = make_session();
    assert!(!check_guard(&Guard::HasLocalSDP, &s).await);
}

#[tokio::test]
async fn test_has_remote_sdp_true() {
    let mut s = make_session();
    s.remote_sdp = Some("v=0\r\n".into());
    assert!(check_guard(&Guard::HasRemoteSDP, &s).await);
}

#[tokio::test]
async fn test_has_remote_sdp_false() {
    let s = make_session();
    assert!(!check_guard(&Guard::HasRemoteSDP, &s).await);
}

#[tokio::test]
async fn test_has_negotiated_config_true() {
    let mut s = make_session();
    s.negotiated_config = Some(NegotiatedConfig {
        local_addr: "127.0.0.1:5000".parse::<SocketAddr>().unwrap(),
        remote_addr: "127.0.0.1:5002".parse::<SocketAddr>().unwrap(),
        codec: "PCMU".into(),
        sample_rate: 8000,
        channels: 1,
    });
    assert!(check_guard(&Guard::HasNegotiatedConfig, &s).await);
}

#[tokio::test]
async fn test_has_negotiated_config_false() {
    let s = make_session();
    assert!(!check_guard(&Guard::HasNegotiatedConfig, &s).await);
}

// ── Readiness-flag guards ───────────────────────────────────────────────────

#[tokio::test]
async fn test_dialog_established_guard() {
    let mut s = make_session();
    assert!(!check_guard(&Guard::DialogEstablished, &s).await);
    s.dialog_established = true;
    assert!(check_guard(&Guard::DialogEstablished, &s).await);
}

#[tokio::test]
async fn test_media_ready_guard() {
    let mut s = make_session();
    assert!(!check_guard(&Guard::MediaReady, &s).await);
    s.media_session_ready = true;
    assert!(check_guard(&Guard::MediaReady, &s).await);
}

#[tokio::test]
async fn test_sdp_negotiated_guard() {
    let mut s = make_session();
    assert!(!check_guard(&Guard::SDPNegotiated, &s).await);
    s.sdp_negotiated = true;
    assert!(check_guard(&Guard::SDPNegotiated, &s).await);
}

#[tokio::test]
async fn test_all_conditions_met_all_true() {
    let mut s = make_session();
    s.dialog_established = true;
    s.media_session_ready = true;
    s.sdp_negotiated = true;
    assert!(check_guard(&Guard::AllConditionsMet, &s).await);
}

#[tokio::test]
async fn test_all_conditions_met_missing_dialog() {
    let mut s = make_session();
    s.media_session_ready = true;
    s.sdp_negotiated = true;
    assert!(!check_guard(&Guard::AllConditionsMet, &s).await);
}

#[tokio::test]
async fn test_all_conditions_met_missing_media() {
    let mut s = make_session();
    s.dialog_established = true;
    s.sdp_negotiated = true;
    assert!(!check_guard(&Guard::AllConditionsMet, &s).await);
}

#[tokio::test]
async fn test_all_conditions_met_missing_sdp() {
    let mut s = make_session();
    s.dialog_established = true;
    s.media_session_ready = true;
    assert!(!check_guard(&Guard::AllConditionsMet, &s).await);
}

#[tokio::test]
async fn test_all_conditions_met_none_set() {
    let s = make_session();
    assert!(!check_guard(&Guard::AllConditionsMet, &s).await);
}

// ── CallState-based guards ──────────────────────────────────────────────────

#[tokio::test]
async fn test_is_idle_guard() {
    let s = make_session(); // default is Idle
    assert!(check_guard(&Guard::IsIdle, &s).await);
}

#[tokio::test]
async fn test_is_idle_guard_when_active() {
    let mut s = make_session();
    s.call_state = CallState::Active;
    assert!(!check_guard(&Guard::IsIdle, &s).await);
}

#[tokio::test]
async fn test_in_active_call_guard() {
    let mut s = make_session();
    s.call_state = CallState::Active;
    assert!(check_guard(&Guard::InActiveCall, &s).await);
}

#[tokio::test]
async fn test_in_active_call_guard_when_idle() {
    let s = make_session();
    assert!(!check_guard(&Guard::InActiveCall, &s).await);
}

#[tokio::test]
async fn test_is_registered_guard() {
    let mut s = make_session();
    s.call_state = CallState::Registered;
    assert!(check_guard(&Guard::IsRegistered, &s).await);
}

#[tokio::test]
async fn test_is_registered_guard_when_registering() {
    let mut s = make_session();
    s.call_state = CallState::Registering;
    assert!(!check_guard(&Guard::IsRegistered, &s).await);
}

#[tokio::test]
async fn test_is_subscribed_guard() {
    let mut s = make_session();
    s.call_state = CallState::Subscribed;
    assert!(check_guard(&Guard::IsSubscribed, &s).await);
}

#[tokio::test]
async fn test_has_active_subscription_guard() {
    let mut s = make_session();
    s.call_state = CallState::Subscribed;
    assert!(check_guard(&Guard::HasActiveSubscription, &s).await);
}

#[tokio::test]
async fn test_has_active_subscription_when_idle() {
    let s = make_session();
    assert!(!check_guard(&Guard::HasActiveSubscription, &s).await);
}

// ── Custom guard ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_custom_guard_returns_false() {
    let s = make_session();
    assert!(!check_guard(&Guard::Custom("my_custom_check".into()), &s).await);
}
