//! RFC 9725 outbound WHIP resource client.
//!
//! Creation is intentionally one-shot: an ambiguous `POST` is never retried.
//! Redirect following is disabled, resolved addresses are checked before being
//! installed into the rustls HTTP client, and the returned resource is bound
//! to the creation origin. `Location` and a strong `ETag` are retained for
//! conditional PATCH/DELETE lifecycle operations.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::Arc;

use reqwest::header::{
    HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, ETAG, IF_MATCH,
    LOCATION,
};
use reqwest::{Client, Method, StatusCode};
use url::Url;
use webrtc::peer_connection::RTCIceCandidateInit;
use zeroize::Zeroize;

use crate::errors::{Result, WebRtcError};
use crate::originate::{WebRtcOriginateContext, WebRtcOriginateContextError};

pub(crate) const SDP_CONTENT_TYPE: &str = "application/sdp";
pub(crate) const TRICKLE_CONTENT_TYPE: &str = "application/trickle-ice-sdpfrag";
const MAX_RESPONSE_BODY_BYTES: usize = 256 * 1024;
const MAX_LOCATION_BYTES: usize = 2_048;
const MAX_ETAG_BYTES: usize = 512;

pub(crate) struct WhipResourceClient {
    client: Client,
    context: Arc<WebRtcOriginateContext>,
    resource_url: Url,
    etag: String,
}

pub(crate) struct WhipCreatedResource {
    pub client: WhipResourceClient,
    pub answer_sdp: String,
}

impl WhipResourceClient {
    pub async fn create(
        context: Arc<WebRtcOriginateContext>,
        offer_sdp: String,
    ) -> Result<WhipCreatedResource> {
        context.validate().map_err(context_error_to_webrtc_error)?;
        let endpoint = context.endpoint();
        let client = build_checked_client(&context, "WHIP").await?;

        let request = client
            .post(endpoint.clone())
            .header(CONTENT_TYPE, SDP_CONTENT_TYPE)
            .header(ACCEPT, SDP_CONTENT_TYPE)
            .body(offer_sdp);
        let request = authorize(request, &context, "WHIP").await?;
        // One attempt only. A timeout after bytes reached the origin is
        // ambiguous and must be reconciled by policy, never replayed here.
        let response = request
            .send()
            .await
            .map_err(|_| WebRtcError::Signaling("WHIP resource creation failed".into()))?;
        if response.status().is_redirection() {
            return Err(WebRtcError::Signaling(
                "WHIP creation redirect was rejected".into(),
            ));
        }
        if response.status() != StatusCode::CREATED {
            tracing::warn!(
                operation = "create",
                status = response.status().as_u16(),
                "WHIP origin rejected operation"
            );
            return Err(WebRtcError::Signaling(
                "WHIP origin rejected resource creation".into(),
            ));
        }
        let resource_url = parse_resource_location(endpoint, response.headers(), "WHIP")?;
        if let Err(error) = require_sdp_content_type(response.headers(), "WHIP") {
            best_effort_delete(&client, &context, &resource_url, "WHIP").await;
            return Err(error);
        }
        let etag = match parse_strong_etag(response.headers(), "WHIP") {
            Ok(etag) => etag,
            Err(error) => {
                best_effort_delete(&client, &context, &resource_url, "WHIP").await;
                return Err(error);
            }
        };
        let answer_sdp = match read_bounded_text(response, "WHIP").await {
            Ok(answer_sdp) => answer_sdp,
            Err(error) => {
                best_effort_delete(&client, &context, &resource_url, "WHIP").await;
                return Err(error);
            }
        };
        if answer_sdp.trim().is_empty() {
            best_effort_delete(&client, &context, &resource_url, "WHIP").await;
            return Err(WebRtcError::Signaling(
                "WHIP creation returned an empty answer".into(),
            ));
        }
        Ok(WhipCreatedResource {
            client: Self {
                client,
                context,
                resource_url,
                etag,
            },
            answer_sdp,
        })
    }

    pub async fn patch_candidate(&mut self, candidate: RTCIceCandidateInit) -> Result<()> {
        let fragment = candidate_to_sdp_fragment(candidate, "WHIP")?;
        self.patch_trickle_fragment(fragment, "candidate").await
    }

    pub async fn patch_ice_complete(&mut self) -> Result<()> {
        self.patch_trickle_fragment("a=end-of-candidates\r\n".into(), "completion")
            .await
    }

    async fn patch_trickle_fragment(
        &mut self,
        fragment: String,
        operation: &'static str,
    ) -> Result<()> {
        let request = self
            .client
            .request(Method::PATCH, self.resource_url.clone())
            .header(CONTENT_TYPE, TRICKLE_CONTENT_TYPE)
            .header(IF_MATCH, self.etag.clone())
            .body(fragment);
        let request = authorize(request, &self.context, "WHIP").await?;
        let response = request
            .send()
            .await
            .map_err(|_| WebRtcError::Signaling(format!("WHIP {operation} PATCH failed")))?;
        if response.status().is_redirection() {
            return Err(WebRtcError::Signaling(
                "WHIP resource redirect was rejected".into(),
            ));
        }
        if !matches!(response.status(), StatusCode::NO_CONTENT | StatusCode::OK) {
            tracing::warn!(
                operation = "candidate-patch",
                status = response.status().as_u16(),
                "WHIP origin rejected operation"
            );
            return Err(WebRtcError::Signaling(
                "WHIP origin rejected candidate PATCH".into(),
            ));
        }
        self.accept_rotated_etag(response.headers())?;
        Ok(())
    }

    pub async fn delete(&mut self) -> Result<()> {
        let request = self
            .client
            .delete(self.resource_url.clone())
            .header(IF_MATCH, self.etag.clone());
        let request = authorize(request, &self.context, "WHIP").await?;
        let response = request
            .send()
            .await
            .map_err(|_| WebRtcError::Signaling("WHIP resource DELETE failed".into()))?;
        if response.status().is_redirection() {
            return Err(WebRtcError::Signaling(
                "WHIP resource redirect was rejected".into(),
            ));
        }
        if !matches!(
            response.status(),
            StatusCode::OK | StatusCode::NO_CONTENT | StatusCode::NOT_FOUND
        ) {
            tracing::warn!(
                operation = "delete",
                status = response.status().as_u16(),
                "WHIP origin rejected operation"
            );
            return Err(WebRtcError::Signaling(
                "WHIP origin rejected resource DELETE".into(),
            ));
        }
        Ok(())
    }

    fn accept_rotated_etag(&mut self, headers: &HeaderMap) -> Result<()> {
        let rotated = parse_strong_etag(headers, "WHIP")?;
        if rotated == self.etag {
            return Err(WebRtcError::Signaling(
                "WHIP origin did not rotate its strong ETag".into(),
            ));
        }
        self.etag = rotated;
        Ok(())
    }
}

pub(crate) async fn build_checked_client(
    context: &WebRtcOriginateContext,
    protocol: &'static str,
) -> Result<Client> {
    let endpoint = context.endpoint();
    let host = endpoint
        .host_str()
        .ok_or_else(|| WebRtcError::Signaling(format!("{protocol} target has no host")))?;
    let port = endpoint
        .port_or_known_default()
        .ok_or_else(|| WebRtcError::Signaling(format!("{protocol} target has no port")))?;
    let policy = context.target_policy();
    let resolved = tokio::time::timeout(
        policy.connect_timeout(),
        tokio::net::lookup_host((host, port)),
    )
    .await
    .map_err(|_| WebRtcError::Timeout("WebRTC HTTP target resolution"))?
    .map_err(|_| WebRtcError::Signaling(format!("{protocol} target resolution failed")))?;
    let mut addresses = BTreeSet::new();
    for address in resolved {
        addresses.insert(address);
        if addresses.len() > policy.max_resolved_addresses() {
            return Err(context_error_to_webrtc_error(
                WebRtcOriginateContextError::TooManyResolvedAddresses,
            ));
        }
    }
    if addresses.is_empty() {
        return Err(WebRtcError::Signaling(format!(
            "{protocol} target resolution returned no addresses"
        )));
    }
    if addresses
        .iter()
        .any(|address| !policy.address_allowed(address.ip()))
    {
        return Err(context_error_to_webrtc_error(
            WebRtcOriginateContextError::AddressForbidden,
        ));
    }
    let addresses: Vec<SocketAddr> = addresses.into_iter().collect();
    let builder = Client::builder()
        .use_rustls_tls()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(policy.connect_timeout())
        .timeout(policy.signaling_timeout())
        .resolve_to_addrs(host, &addresses);
    #[cfg(feature = "tls-rustls")]
    let builder = match context.tls_trust() {
        Some(trust) => {
            let mut configured = builder;
            for certificate in trust.certificates() {
                let certificate =
                    reqwest::Certificate::from_der(certificate.as_ref()).map_err(|_| {
                        WebRtcError::Signaling(format!(
                            "{protocol} TLS trust profile construction failed"
                        ))
                    })?;
                configured = configured.add_root_certificate(certificate);
            }
            configured
        }
        None => builder,
    };
    builder
        .build()
        .map_err(|_| WebRtcError::Signaling(format!("{protocol} HTTP client construction failed")))
}

pub(crate) async fn authorize(
    request: reqwest::RequestBuilder,
    context: &WebRtcOriginateContext,
    protocol: &'static str,
) -> Result<reqwest::RequestBuilder> {
    let credential = context
        .bearer_credential()
        .await
        .map_err(context_error_to_webrtc_error)?;
    let Some(credential) = credential else {
        return Ok(request);
    };
    let mut value = String::with_capacity("Bearer ".len() + credential.expose_secret().len());
    value.push_str("Bearer ");
    value.push_str(credential.expose_secret());
    let mut header = HeaderValue::from_str(&value).map_err(|_| {
        WebRtcError::Signaling(format!("{protocol} authorization header is invalid"))
    })?;
    header.set_sensitive(true);
    value.zeroize();
    Ok(request.header(AUTHORIZATION, header))
}

pub(crate) async fn best_effort_delete(
    client: &Client,
    context: &WebRtcOriginateContext,
    resource_url: &Url,
    protocol: &'static str,
) {
    let request = client.delete(resource_url.clone());
    let Ok(request) = authorize(request, context, protocol).await else {
        return;
    };
    let _ = request.send().await;
}

pub(crate) fn parse_resource_location(
    base: &Url,
    headers: &HeaderMap,
    protocol: &'static str,
) -> Result<Url> {
    let raw = headers
        .get(LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            WebRtcError::Signaling(format!("{protocol} Location is missing or invalid"))
        })?;
    if raw.is_empty() || raw.len() > MAX_LOCATION_BYTES {
        return Err(WebRtcError::Signaling(format!(
            "{protocol} Location is missing or invalid"
        )));
    }
    let resource = base
        .join(raw)
        .map_err(|_| WebRtcError::Signaling(format!("{protocol} Location is invalid")))?;
    if !resource.username().is_empty()
        || resource.password().is_some()
        || resource.query().is_some()
        || resource.fragment().is_some()
    {
        return Err(WebRtcError::Signaling(format!(
            "{protocol} Location contains forbidden metadata"
        )));
    }
    if resource.scheme() != base.scheme()
        || resource.host_str() != base.host_str()
        || resource.port_or_known_default() != base.port_or_known_default()
    {
        return Err(WebRtcError::Signaling(format!(
            "{protocol} Location crossed the credential origin boundary"
        )));
    }
    Ok(resource)
}

pub(crate) fn parse_strong_etag(headers: &HeaderMap, protocol: &'static str) -> Result<String> {
    let etag = headers
        .get(ETAG)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| WebRtcError::Signaling(format!("{protocol} strong ETag is missing")))?;
    if etag.len() < 2
        || etag.len() > MAX_ETAG_BYTES
        || etag.starts_with("W/")
        || !etag.starts_with('"')
        || !etag.ends_with('"')
        || etag.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(WebRtcError::Signaling(format!(
            "{protocol} strong ETag is invalid"
        )));
    }
    Ok(etag.to_owned())
}

pub(crate) fn require_sdp_content_type(headers: &HeaderMap, protocol: &'static str) -> Result<()> {
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    if content_type != Some(SDP_CONTENT_TYPE) {
        return Err(WebRtcError::Signaling(format!(
            "{protocol} SDP Content-Type is invalid"
        )));
    }
    Ok(())
}

pub(crate) async fn read_bounded_text(
    mut response: reqwest::Response,
    protocol: &'static str,
) -> Result<String> {
    if response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > MAX_RESPONSE_BODY_BYTES)
    {
        return Err(WebRtcError::Signaling(format!(
            "{protocol} SDP body exceeds its bound"
        )));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| WebRtcError::Signaling(format!("{protocol} SDP body failed")))?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BODY_BYTES {
            return Err(WebRtcError::Signaling(format!(
                "{protocol} SDP body exceeds its bound"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body)
        .map_err(|_| WebRtcError::Signaling(format!("{protocol} SDP body is not UTF-8")))
}

pub(crate) fn candidate_to_sdp_fragment(
    candidate: RTCIceCandidateInit,
    protocol: &'static str,
) -> Result<String> {
    if candidate.candidate.is_empty()
        || candidate.candidate.len() > 16 * 1024
        || candidate
            .candidate
            .bytes()
            .any(|byte| matches!(byte, b'\r' | b'\n'))
    {
        return Err(WebRtcError::Signaling(format!(
            "{protocol} ICE candidate is invalid"
        )));
    }
    let mut fragment = String::new();
    if let Some(mid) = candidate.sdp_mid {
        if mid.len() > 256 || mid.bytes().any(|byte| byte.is_ascii_control()) {
            return Err(WebRtcError::Signaling(format!(
                "{protocol} ICE candidate mid is invalid"
            )));
        }
        // webrtc-rs emits `Some("")` when the initial SDP has no explicit
        // MID. Omit that empty attribute; `a=mid:` would be invalid.
        if !mid.is_empty() {
            fragment.push_str("a=mid:");
            fragment.push_str(&mid);
            fragment.push_str("\r\n");
        }
    }
    fragment.push_str("a=");
    fragment.push_str(&candidate.candidate);
    fragment.push_str("\r\n");
    Ok(fragment)
}

fn context_error_to_webrtc_error(error: WebRtcOriginateContextError) -> WebRtcError {
    WebRtcError::InvalidArgument(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::StatusCode as AxumStatusCode;
    use axum::response::IntoResponse;
    use axum::routing::{patch, post};
    use axum::Router;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    #[derive(Clone, Default)]
    struct ConditionalState {
        seen: Arc<Mutex<Vec<String>>>,
    }

    #[derive(Clone)]
    struct RedirectState {
        location: String,
        origin_saw_credential: Arc<AtomicBool>,
    }

    async fn redirect_creation(
        State(state): State<RedirectState>,
        headers: axum::http::HeaderMap,
    ) -> impl IntoResponse {
        state.origin_saw_credential.store(
            headers
                .get(AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                == Some("Bearer redirect-canary"),
            Ordering::SeqCst,
        );
        (
            AxumStatusCode::TEMPORARY_REDIRECT,
            [(LOCATION, state.location)],
        )
    }

    async fn redirected_target(State(state): State<Arc<AtomicUsize>>) -> impl IntoResponse {
        state.fetch_add(1, Ordering::SeqCst);
        AxumStatusCode::INTERNAL_SERVER_ERROR
    }

    async fn conditional_patch(
        State(state): State<ConditionalState>,
        headers: axum::http::HeaderMap,
    ) -> impl IntoResponse {
        state.seen.lock().unwrap().push(
            headers
                .get("if-match")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned(),
        );
        (
            AxumStatusCode::NO_CONTENT,
            [("etag", HeaderValue::from_static("\"version-2\""))],
        )
    }

    async fn conditional_delete(
        State(state): State<ConditionalState>,
        headers: axum::http::HeaderMap,
    ) -> impl IntoResponse {
        state.seen.lock().unwrap().push(
            headers
                .get("if-match")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned(),
        );
        AxumStatusCode::NO_CONTENT
    }

    #[test]
    fn resource_location_must_stay_on_the_creation_origin() {
        let base = Url::parse("https://media.example.test/whip/channel").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(LOCATION, HeaderValue::from_static("/whip/resource-1"));
        assert_eq!(
            parse_resource_location(&base, &headers, "WHIP")
                .unwrap()
                .as_str(),
            "https://media.example.test/whip/resource-1"
        );
        headers.insert(
            LOCATION,
            HeaderValue::from_static("https://other.example.test/resource"),
        );
        assert!(parse_resource_location(&base, &headers, "WHIP").is_err());
        headers.insert(
            LOCATION,
            HeaderValue::from_static("http://media.example.test/resource"),
        );
        assert!(parse_resource_location(&base, &headers, "WHIP").is_err());
    }

    #[test]
    fn etag_must_be_strong_and_bounded() {
        let mut headers = HeaderMap::new();
        headers.insert(ETAG, HeaderValue::from_static("\"version-1\""));
        assert_eq!(
            parse_strong_etag(&headers, "WHIP").unwrap(),
            "\"version-1\""
        );
        headers.insert(ETAG, HeaderValue::from_static("W/\"version-1\""));
        assert!(parse_strong_etag(&headers, "WHIP").is_err());
    }

    #[test]
    fn candidate_fragment_rejects_line_injection() {
        assert!(candidate_to_sdp_fragment(
            RTCIceCandidateInit {
                candidate: "candidate:ok\r\na=evil".into(),
                ..Default::default()
            },
            "WHIP"
        )
        .is_err());
    }

    #[tokio::test]
    async fn patch_and_delete_are_conditional_and_track_rotated_etag() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = ConditionalState::default();
        let server_state = state.clone();
        let app = Router::new()
            .route(
                "/resource",
                patch(conditional_patch).delete(conditional_delete),
            )
            .with_state(server_state);
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let policy = crate::originate::WebRtcTargetPolicy::default()
            .allow_port(address.port())
            .allow_insecure(true)
            .allow_loopback(true);
        let context = Arc::new(
            WebRtcOriginateContext::new(
                format!("http://{address}/whip"),
                crate::originate::WebRtcSignalingMode::Whip,
                crate::originate::WebRtcIceExchangePolicy::Trickle,
                policy,
                None,
            )
            .unwrap(),
        );
        let mut resource = WhipResourceClient {
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            context,
            resource_url: Url::parse(&format!("http://{address}/resource")).unwrap(),
            etag: "\"version-1\"".into(),
        };
        resource
            .patch_candidate(RTCIceCandidateInit {
                candidate: "candidate:1 1 udp 1 127.0.0.1 9 typ host".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        resource.delete().await.unwrap();
        assert_eq!(
            *state.seen.lock().unwrap(),
            vec!["\"version-1\"", "\"version-2\""]
        );
        server.abort();
    }

    #[tokio::test]
    async fn creation_redirect_is_not_followed_or_forwarded_a_credential() {
        let target_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_address = target_listener.local_addr().unwrap();
        let redirected_requests = Arc::new(AtomicUsize::new(0));
        let target_state = Arc::clone(&redirected_requests);
        let target = Router::new()
            .route("/capture", post(redirected_target))
            .with_state(target_state);
        let target_server = tokio::spawn(async move {
            let _ = axum::serve(target_listener, target).await;
        });

        let origin_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_address = origin_listener.local_addr().unwrap();
        let origin_saw_credential = Arc::new(AtomicBool::new(false));
        let origin_state = RedirectState {
            location: format!("http://{target_address}/capture"),
            origin_saw_credential: Arc::clone(&origin_saw_credential),
        };
        let origin = Router::new()
            .route("/whip", post(redirect_creation))
            .with_state(origin_state);
        let origin_server = tokio::spawn(async move {
            let _ = axum::serve(origin_listener, origin).await;
        });

        let policy = crate::originate::WebRtcTargetPolicy::default()
            .allow_port(origin_address.port())
            .allow_insecure(true)
            .allow_loopback(true);
        let provider = Arc::new(crate::originate::StaticWebRtcBearerCredentialProvider::new(
            crate::originate::WebRtcBearerCredential::new("redirect-canary").unwrap(),
        ));
        let context = Arc::new(
            WebRtcOriginateContext::new(
                format!("http://{origin_address}/whip"),
                crate::originate::WebRtcSignalingMode::Whip,
                crate::originate::WebRtcIceExchangePolicy::FullGather,
                policy,
                Some(provider),
            )
            .unwrap(),
        );

        let result = WhipResourceClient::create(context, "v=0\r\n".into()).await;
        assert!(result.is_err());
        assert!(origin_saw_credential.load(Ordering::SeqCst));
        assert_eq!(redirected_requests.load(Ordering::SeqCst), 0);
        origin_server.abort();
        target_server.abort();
    }
}
