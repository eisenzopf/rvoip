use rvoip_client::Credential;

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
