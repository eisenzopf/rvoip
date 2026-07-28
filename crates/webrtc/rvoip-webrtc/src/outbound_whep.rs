//! WHEP draft-04 outbound playback resource client.
//!
//! The initial `POST` is deliberately one-shot. A successful origin may
//! either accept the player's offer with `201 Created`, or return a
//! `406 Not Acceptable` server counter-offer which is completed by one
//! `application/sdp` PATCH before its advertised expiry. Redirects are never
//! followed and every retained resource remains on the credential origin.

use std::sync::Arc;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use reqwest::header::{HeaderMap, ACCEPT, CONTENT_TYPE, IF_MATCH};
use reqwest::{Client, Method, StatusCode};
use url::Url;
use webrtc::peer_connection::RTCIceCandidateInit;

use crate::errors::{Result, WebRtcError};
use crate::originate::WebRtcOriginateContext;
use crate::outbound_whip::{
    authorize, best_effort_delete, build_checked_client, candidate_to_sdp_fragment,
    parse_resource_location, parse_strong_etag, read_bounded_text, require_sdp_content_type,
    SDP_CONTENT_TYPE, TRICKLE_CONTENT_TYPE,
};

const PROTOCOL: &str = "WHEP";
const DEFAULT_COUNTER_OFFER_LIFETIME_SECONDS: i64 = 30;
const MAX_CONTENT_TYPE_BYTES: usize = 2_048;

pub(crate) struct WhepResourceClient {
    client: Client,
    context: Arc<WebRtcOriginateContext>,
    resource_url: Url,
    etag: String,
}

pub(crate) enum WhepCreation {
    Answer(WhepCreatedResource),
    CounterOffer(WhepCounterOffer),
}

pub(crate) struct WhepCreatedResource {
    pub client: WhepResourceClient,
    pub answer_sdp: String,
}

pub(crate) struct WhepCounterOffer {
    pub client: WhepResourceClient,
    pub offer_sdp: String,
    expires_at: DateTime<Utc>,
}

impl WhepCounterOffer {
    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }

    pub async fn complete(&mut self, answer_sdp: String) -> Result<()> {
        if self.is_expired() {
            return Err(WebRtcError::Signaling(
                "WHEP counter-offer expired before its answer".into(),
            ));
        }
        self.client.patch_counter_offer_answer(answer_sdp).await?;
        Ok(())
    }
}

impl WhepResourceClient {
    pub async fn create(
        context: Arc<WebRtcOriginateContext>,
        offer_sdp: String,
    ) -> Result<WhepCreation> {
        context
            .validate()
            .map_err(|error| WebRtcError::InvalidArgument(error.to_string()))?;
        let endpoint = context.endpoint();
        let client = build_checked_client(&context, PROTOCOL).await?;
        let request = client
            .post(endpoint.clone())
            .header(CONTENT_TYPE, SDP_CONTENT_TYPE)
            .header(ACCEPT, SDP_CONTENT_TYPE)
            .body(offer_sdp);
        let request = authorize(request, &context, PROTOCOL).await?;

        // Never replay this POST. If the response is lost, whether a resource
        // was allocated is ambiguous and must be reconciled outside this
        // transport primitive.
        let response = request
            .send()
            .await
            .map_err(|_| WebRtcError::Signaling("WHEP resource creation failed".into()))?;
        if response.status().is_redirection() {
            return Err(WebRtcError::Signaling(
                "WHEP creation redirect was rejected".into(),
            ));
        }

        match response.status() {
            StatusCode::CREATED => {
                let resource_url = parse_resource_location(endpoint, response.headers(), PROTOCOL)?;
                if let Err(error) = require_sdp_content_type(response.headers(), PROTOCOL) {
                    best_effort_delete(&client, &context, &resource_url, PROTOCOL).await;
                    return Err(error);
                }
                // Draft-04 requires the initial ICE entity-tag on the 201 path.
                let etag = match parse_strong_etag(response.headers(), PROTOCOL) {
                    Ok(etag) => etag,
                    Err(error) => {
                        best_effort_delete(&client, &context, &resource_url, PROTOCOL).await;
                        return Err(error);
                    }
                };
                let answer_sdp = match read_bounded_text(response, PROTOCOL).await {
                    Ok(answer_sdp) => answer_sdp,
                    Err(error) => {
                        best_effort_delete(&client, &context, &resource_url, PROTOCOL).await;
                        return Err(error);
                    }
                };
                if let Err(error) = require_nonempty_sdp(&answer_sdp, "answer") {
                    best_effort_delete(&client, &context, &resource_url, PROTOCOL).await;
                    return Err(error);
                }
                Ok(WhepCreation::Answer(WhepCreatedResource {
                    client: Self {
                        client,
                        context,
                        resource_url,
                        etag,
                    },
                    answer_sdp,
                }))
            }
            StatusCode::NOT_ACCEPTABLE => {
                let resource_url = parse_resource_location(endpoint, response.headers(), PROTOCOL)?;
                if let Err(error) = require_sdp_content_type(response.headers(), PROTOCOL) {
                    best_effort_delete(&client, &context, &resource_url, PROTOCOL).await;
                    return Err(error);
                }
                let expires_at = match parse_counter_offer_expiry(response.headers()) {
                    Ok(expires_at) => expires_at,
                    Err(error) => {
                        best_effort_delete(&client, &context, &resource_url, PROTOCOL).await;
                        return Err(error);
                    }
                };
                let etag = match parse_strong_etag(response.headers(), PROTOCOL) {
                    Ok(etag) => etag,
                    Err(error) => {
                        best_effort_delete(&client, &context, &resource_url, PROTOCOL).await;
                        return Err(error);
                    }
                };
                let offer_sdp = match read_bounded_text(response, PROTOCOL).await {
                    Ok(offer_sdp) => offer_sdp,
                    Err(error) => {
                        best_effort_delete(&client, &context, &resource_url, PROTOCOL).await;
                        return Err(error);
                    }
                };
                if let Err(error) = require_nonempty_sdp(&offer_sdp, "counter-offer") {
                    best_effort_delete(&client, &context, &resource_url, PROTOCOL).await;
                    return Err(error);
                }
                let counter_offer = WhepCounterOffer {
                    client: Self {
                        client,
                        context,
                        resource_url,
                        etag,
                    },
                    offer_sdp,
                    expires_at,
                };
                if counter_offer.is_expired() {
                    let error =
                        WebRtcError::Signaling("WHEP counter-offer was already expired".into());
                    best_effort_delete(
                        &counter_offer.client.client,
                        &counter_offer.client.context,
                        &counter_offer.client.resource_url,
                        PROTOCOL,
                    )
                    .await;
                    return Err(error);
                }
                Ok(WhepCreation::CounterOffer(counter_offer))
            }
            status => {
                tracing::warn!(
                    operation = "create",
                    status = status.as_u16(),
                    "WHEP origin rejected operation"
                );
                Err(WebRtcError::Signaling(
                    "WHEP origin rejected resource creation".into(),
                ))
            }
        }
    }

    async fn patch_counter_offer_answer(&mut self, answer_sdp: String) -> Result<()> {
        require_nonempty_sdp(&answer_sdp, "counter-offer answer")?;
        let request = self
            .client
            .request(Method::PATCH, self.resource_url.clone())
            .header(CONTENT_TYPE, SDP_CONTENT_TYPE)
            .header(IF_MATCH, self.etag.clone())
            .body(answer_sdp);
        let request = authorize(request, &self.context, PROTOCOL).await?;
        let response = request
            .send()
            .await
            .map_err(|_| WebRtcError::Signaling("WHEP counter-offer answer PATCH failed".into()))?;
        if response.status().is_redirection() {
            return Err(WebRtcError::Signaling(
                "WHEP resource redirect was rejected".into(),
            ));
        }
        if response.status() != StatusCode::NO_CONTENT {
            tracing::warn!(
                operation = "counter-offer-answer-patch",
                status = response.status().as_u16(),
                "WHEP origin rejected operation"
            );
            return Err(WebRtcError::Signaling(
                "WHEP origin rejected counter-offer answer PATCH".into(),
            ));
        }
        self.accept_rotated_etag(response.headers())?;
        Ok(())
    }

    pub async fn patch_candidate(&mut self, candidate: RTCIceCandidateInit) -> Result<()> {
        let fragment = candidate_to_sdp_fragment(candidate, PROTOCOL)?;
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
        let request = authorize(request, &self.context, PROTOCOL).await?;
        let response = request
            .send()
            .await
            .map_err(|_| WebRtcError::Signaling(format!("WHEP {operation} PATCH failed")))?;
        if response.status().is_redirection() {
            return Err(WebRtcError::Signaling(
                "WHEP resource redirect was rejected".into(),
            ));
        }
        if response.status() != StatusCode::NO_CONTENT {
            tracing::warn!(
                operation = "candidate-patch",
                status = response.status().as_u16(),
                "WHEP origin rejected operation"
            );
            return Err(WebRtcError::Signaling(
                "WHEP origin rejected candidate PATCH".into(),
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
        let request = authorize(request, &self.context, PROTOCOL).await?;
        let response = request
            .send()
            .await
            .map_err(|_| WebRtcError::Signaling("WHEP resource DELETE failed".into()))?;
        if response.status().is_redirection() {
            return Err(WebRtcError::Signaling(
                "WHEP resource redirect was rejected".into(),
            ));
        }
        if !matches!(
            response.status(),
            StatusCode::OK | StatusCode::NO_CONTENT | StatusCode::NOT_FOUND
        ) {
            tracing::warn!(
                operation = "delete",
                status = response.status().as_u16(),
                "WHEP origin rejected operation"
            );
            return Err(WebRtcError::Signaling(
                "WHEP origin rejected resource DELETE".into(),
            ));
        }
        Ok(())
    }

    fn accept_rotated_etag(&mut self, headers: &HeaderMap) -> Result<()> {
        let rotated = parse_strong_etag(headers, PROTOCOL)?;
        if rotated == self.etag {
            return Err(WebRtcError::Signaling(
                "WHEP origin did not rotate its strong ETag".into(),
            ));
        }
        self.etag = rotated;
        Ok(())
    }
}

fn require_nonempty_sdp(sdp: &str, kind: &'static str) -> Result<()> {
    if sdp.trim().is_empty() {
        return Err(WebRtcError::Signaling(format!(
            "WHEP creation returned an empty {kind}"
        )));
    }
    Ok(())
}

fn parse_counter_offer_expiry(headers: &HeaderMap) -> Result<DateTime<Utc>> {
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| WebRtcError::Signaling("WHEP Content-Type is invalid".into()))?;
    if content_type.len() > MAX_CONTENT_TYPE_BYTES || !content_type.is_ascii() {
        return Err(WebRtcError::Signaling(
            "WHEP Content-Type is invalid".into(),
        ));
    }
    for parameter in content_type.split(';').skip(1) {
        let Some((name, value)) = parameter.trim().split_once('=') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case("valid-until") {
            continue;
        }
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .unwrap_or(value);
        if value.is_empty() || value.bytes().any(|byte| byte.is_ascii_control()) {
            return Err(WebRtcError::Signaling(
                "WHEP counter-offer expiry is invalid".into(),
            ));
        }
        let parsed = DateTime::parse_from_rfc2822(value)
            .map_err(|_| WebRtcError::Signaling("WHEP counter-offer expiry is invalid".into()))?;
        if parsed.offset().local_minus_utc() != 0 {
            return Err(WebRtcError::Signaling(
                "WHEP counter-offer expiry is invalid".into(),
            ));
        }
        return Ok(parsed.with_timezone(&Utc));
    }
    Ok(Utc::now() + ChronoDuration::seconds(DEFAULT_COUNTER_OFFER_LIFETIME_SECONDS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::StatusCode as AxumStatusCode;
    use axum::response::IntoResponse;
    use axum::routing::{delete, post};
    use axum::Router;
    use reqwest::header::{HeaderValue, LOCATION};
    use std::sync::atomic::{AtomicBool, Ordering};

    async fn incomplete_created_response() -> impl IntoResponse {
        (
            AxumStatusCode::CREATED,
            [(CONTENT_TYPE, SDP_CONTENT_TYPE), (LOCATION, "/resource")],
            "v=0\r\n",
        )
    }

    async fn capture_cleanup_delete(State(deleted): State<Arc<AtomicBool>>) -> impl IntoResponse {
        deleted.store(true, Ordering::SeqCst);
        AxumStatusCode::OK
    }

    #[test]
    fn counter_offer_expiry_defaults_to_thirty_seconds() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(SDP_CONTENT_TYPE));
        let before = Utc::now() + ChronoDuration::seconds(29);
        let expiry = parse_counter_offer_expiry(&headers).unwrap();
        let after = Utc::now() + ChronoDuration::seconds(31);
        assert!(expiry > before);
        assert!(expiry < after);
    }

    #[test]
    fn counter_offer_expiry_accepts_http_date_and_rejects_malformed_values() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static(
                "application/sdp; valid-until=\"Wed, 09 Oct 2030 10:00:00 GMT\"",
            ),
        );
        assert_eq!(
            parse_counter_offer_expiry(&headers).unwrap().to_rfc2822(),
            "Wed, 9 Oct 2030 10:00:00 +0000"
        );
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/sdp; valid-until=never"),
        );
        assert!(parse_counter_offer_expiry(&headers).is_err());
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static(
                "application/sdp; valid-until=\"Wed, 09 Oct 2030 10:00:00 -0700\"",
            ),
        );
        assert!(parse_counter_offer_expiry(&headers).is_err());
    }

    #[tokio::test]
    async fn incomplete_created_resource_is_cleaned_up() {
        let deleted = Arc::new(AtomicBool::new(false));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new()
            .route("/whep", post(incomplete_created_response))
            .route("/resource", delete(capture_cleanup_delete))
            .with_state(Arc::clone(&deleted));
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let policy = crate::originate::WebRtcTargetPolicy::default()
            .allow_port(address.port())
            .allow_insecure(true)
            .allow_loopback(true);
        let context = Arc::new(
            WebRtcOriginateContext::new(
                format!("http://{address}/whep"),
                crate::originate::WebRtcSignalingMode::Whep,
                crate::originate::WebRtcIceExchangePolicy::FullGather,
                policy,
                None,
            )
            .unwrap(),
        );
        assert!(WhepResourceClient::create(context, "v=0\r\n".into())
            .await
            .is_err());
        assert!(deleted.load(Ordering::SeqCst));
        server.abort();
    }
}
