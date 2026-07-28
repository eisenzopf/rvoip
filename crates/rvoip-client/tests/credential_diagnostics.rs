use rvoip_client::{CallTarget, ClientError, Credential, InboundEvent};
use rvoip_core_traits::ids::{ConversationId, MessageId};

const CANARY: &str = "client-credential-canary\r\nAuthorization: exposed";

#[test]
fn client_credentials_keep_values_but_redact_tokens_and_proofs() {
    let credentials = [
        Credential::Bearer(CANARY.into()),
        Credential::OAuth2Dpop {
            access_token: CANARY.into(),
            dpop_proof: CANARY.into(),
        },
    ];
    for credential in credentials {
        let rendered = format!("{credential:?}");
        assert!(!rendered.contains(CANARY), "credential leaked: {rendered}");
        match credential {
            Credential::Bearer(token) => assert_eq!(token, CANARY),
            Credential::OAuth2Dpop {
                access_token,
                dpop_proof,
            } => {
                assert_eq!(access_token, CANARY);
                assert_eq!(dpop_proof, CANARY);
            }
        }
    }
}

#[test]
fn targets_events_and_errors_keep_live_values_out_of_diagnostics() {
    let target = CallTarget::Uri(CANARY.into());
    let event = InboundEvent::Message {
        conversation_id: ConversationId::from_string(CANARY),
        message_id: MessageId::from_string(CANARY),
        from: CANARY.into(),
        body: CANARY.into(),
    };
    let error = ClientError::Protocol(CANARY.into());

    for rendered in [
        format!("{target:?}"),
        format!("{event:?}"),
        format!("{error:?} {error}"),
    ] {
        assert!(!rendered.contains(CANARY), "credential leaked: {rendered}");
    }
    assert!(matches!(target, CallTarget::Uri(value) if value == CANARY));
    assert!(matches!(error, ClientError::Protocol(value) if value == CANARY));
}
