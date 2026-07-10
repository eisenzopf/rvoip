use chrono::{Duration, Utc};
use rvoip_core::events::Event;
use rvoip_core::identity::{AuthenticatedPrincipal, AuthenticationMethod, IdentityAssurance};
use rvoip_core::ConnectionId;
use rvoip_infra_common::events::cross_crate::{RvoipCoreCrossCrateEvent, RvoipCrossCrateEvent};

#[test]
fn principal_cross_crate_event_redacts_tenant_authorization_context() {
    let connection_id = ConnectionId::new();
    let expiry = Utc::now() + Duration::minutes(5);
    let event = Event::ConnectionPrincipalAuthenticated {
        connection_id: connection_id.clone(),
        participant_id: "part_alice".into(),
        principal: AuthenticatedPrincipal {
            subject: "user-42".into(),
            tenant: Some("tenant-a".into()),
            scopes: vec!["calls:read".into(), "calls:write".into()],
            issuer: Some("https://issuer.example".into()),
            expires_at: Some(expiry),
            method: AuthenticationMethod::Jwt,
            assurance: IdentityAssurance::Anonymous,
        },
        at: Utc::now(),
    };

    let wire = event.to_cross_crate();
    let encoded = serde_json::to_string(&wire).expect("serialize cross-crate event");
    let RvoipCrossCrateEvent::Core(RvoipCoreCrossCrateEvent::IdentityAssuranceChanged {
        connection_id: wire_connection_id,
        identity_id,
    }) = wire
    else {
        panic!("expected redacted authentication lifecycle event");
    };

    assert_eq!(wire_connection_id, connection_id.to_string());
    assert_eq!(identity_id, None);
    for sensitive in [
        "user-42",
        "tenant-a",
        "calls:read",
        "calls:write",
        "https://issuer.example",
        "part_alice",
    ] {
        assert!(!encoded.contains(sensitive));
    }
}
