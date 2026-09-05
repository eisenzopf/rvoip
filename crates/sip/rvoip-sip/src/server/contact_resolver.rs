//! AOR → live Contact URI resolution.
//!
//! Per CARVE_PLAN §3 (ContactResolver row): lifted from
//! `orchestration-core/src/traits.rs:81-198` with a SIP-flavored
//! [`ContactRequest`] input (the workforce-flavored `Agent` parameter stays
//! in orchestration-core). The two impls — [`StaticContactResolver`] and
//! [`RegistrarContactResolver`] — preserve the proven behavior. The latter
//! delegates AOR lookups to a [`rvoip_sip_registrar::RegistrarService`] so
//! B2BUA originate-legs can locate live contacts registered elsewhere in the
//! system.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rvoip_sip_registrar::{AddressOfRecord, RegisteredFlowRoute, RegistrarService};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;

/// Input to a [`ContactResolver`]. Either a literal SIP URI to dial, or an
/// AOR to look up against a registrar.
#[derive(Clone, Debug)]
pub enum ContactRequest {
    /// Use a literal SIP URI as-is (no registrar lookup).
    Static {
        /// Literal SIP URI to dial.
        uri: String,
    },
    /// Look up a registered SIP user by AOR; the resolver consults the
    /// configured [`rvoip_sip_registrar::RegistrarService`] to find the live
    /// Contact binding.
    Registered {
        /// Address-of-record to resolve to a live Contact.
        aor: String,
    },
}

/// A resolved live Contact for an AOR or literal URI, with the SIP routing
/// metadata gathered during resolution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedContact {
    /// Contact URI to dial.
    pub uri: String,
    /// Expiry of the registration binding, when known (registrar-sourced).
    pub expires_at: Option<DateTime<Utc>>,
    /// How this contact was resolved (static, registrar, or custom).
    pub source: ContactSource,
    /// Transport associated with the binding (e.g. `Udp`, `Tcp`, `Tls`).
    pub transport: Option<String>,
    /// Source address observed at registration time (`received` parameter, RFC 3261).
    pub received: Option<String>,
    /// Path vector recorded for the binding (RFC 3327 Path).
    pub path: Vec<String>,
    /// SIP instance ID of the registering UA (`+sip.instance`, RFC 5626/5627).
    pub instance_id: Option<String>,
    /// Registration flow identifier (`reg-id`, RFC 5626 outbound).
    pub reg_id: Option<u32>,
    /// Legacy diagnostic field. Opaque registered-flow tokens are never
    /// returned through this serializable projection.
    pub flow_id: Option<String>,
    /// Verified process-local routes used only by the SIP application facade.
    #[serde(skip)]
    pub registered_flow_routes: Vec<RegisteredFlowRoute>,
}

/// Origin of a [`ResolvedContact`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactSource {
    /// Resolved from a literal [`ContactRequest::Static`] URI.
    Static,
    /// Resolved from a live registrar binding.
    Registrar,
    /// Resolved by a custom resolver implementation.
    Custom,
}

/// Error returned when contact resolution fails.
#[derive(Debug, Error)]
pub enum ContactResolverError {
    /// Resolution failed for an unspecified or resolver-defined reason.
    #[error("contact resolution failed: {0}")]
    Failed(String),
    /// The underlying registrar lookup returned an error.
    #[error("registrar error: {0}")]
    Registrar(String),
    /// The supplied AOR could not be parsed.
    #[error("invalid AOR `{aor}`: {detail}")]
    InvalidAor {
        /// The AOR string that failed to parse.
        aor: String,
        /// Parser-supplied detail describing why it is invalid.
        detail: String,
    },
    /// The AOR parsed but has no live (unexpired) contacts registered.
    #[error("AOR `{0}` has no live contacts")]
    NoLiveContacts(String),
}

/// Resolves a [`ContactRequest`] into a live [`ResolvedContact`].
#[async_trait]
pub trait ContactResolver: Send + Sync {
    /// Resolve `request` into a [`ResolvedContact`].
    ///
    /// Returns a [`ContactResolverError`] when the request cannot be
    /// resolved — for example an unsupported request kind, a registrar
    /// failure, an unparseable AOR, or an AOR with no live contacts.
    async fn resolve_contact(
        &self,
        request: &ContactRequest,
    ) -> Result<ResolvedContact, ContactResolverError>;
}

/// Resolves [`ContactRequest::Static`] inputs as-is; rejects
/// [`ContactRequest::Registered`] (no registrar configured). Useful for
/// SIP-only deployments without a registrar.
#[derive(Clone, Debug, Default)]
pub struct StaticContactResolver;

#[async_trait]
impl ContactResolver for StaticContactResolver {
    async fn resolve_contact(
        &self,
        request: &ContactRequest,
    ) -> Result<ResolvedContact, ContactResolverError> {
        match request {
            ContactRequest::Static { uri } => Ok(ResolvedContact {
                uri: uri.clone(),
                expires_at: None,
                source: ContactSource::Static,
                transport: None,
                received: None,
                path: Vec::new(),
                instance_id: None,
                reg_id: None,
                flow_id: None,
                registered_flow_routes: Vec::new(),
            }),
            ContactRequest::Registered { aor } => Err(ContactResolverError::Failed(format!(
                "no registrar-backed resolver configured for AOR {aor}"
            ))),
        }
    }
}

/// Resolves [`ContactRequest::Static`] as-is and [`ContactRequest::Registered`]
/// by querying the configured `RegistrarService` for live contacts.
#[derive(Clone)]
pub struct RegistrarContactResolver {
    registrar: Arc<RegistrarService>,
}

impl RegistrarContactResolver {
    /// Create a resolver backed by the given [`RegistrarService`].
    pub fn new(registrar: Arc<RegistrarService>) -> Self {
        Self { registrar }
    }
}

#[async_trait]
impl ContactResolver for RegistrarContactResolver {
    async fn resolve_contact(
        &self,
        request: &ContactRequest,
    ) -> Result<ResolvedContact, ContactResolverError> {
        match request {
            ContactRequest::Static { uri } => Ok(ResolvedContact {
                uri: uri.clone(),
                expires_at: None,
                source: ContactSource::Static,
                transport: None,
                received: None,
                path: Vec::new(),
                instance_id: None,
                reg_id: None,
                flow_id: None,
                registered_flow_routes: Vec::new(),
            }),
            ContactRequest::Registered { aor: aor_str } => {
                let aor = AddressOfRecord::parse(aor_str).map_err(|error| {
                    ContactResolverError::InvalidAor {
                        aor: aor_str.clone(),
                        detail: error.to_string(),
                    }
                })?;
                let contacts = self
                    .registrar
                    .lookup_live_contacts(&aor, "INVITE")
                    .await
                    .map_err(|error| ContactResolverError::Registrar(error.to_string()))?;

                let mut selected = None;
                let mut registered_flow_routes = Vec::new();
                for contact in contacts {
                    if let Some(flow_token) = contact.flow_id.as_deref() {
                        match self.registrar.resolve_registered_flow(&aor, &contact).await {
                            Ok(route) => {
                                selected.get_or_insert_with(|| contact.clone());
                                registered_flow_routes.push(route);
                            }
                            Err(_) => {
                                let _ = self
                                    .registrar
                                    .set_registered_flow_reachability(
                                        &aor,
                                        flow_token,
                                        rvoip_sip_registrar::ContactReachability::Unreachable,
                                    )
                                    .await;
                            }
                        }
                    } else if selected.is_none() {
                        selected = Some(contact);
                        break;
                    }
                }

                let Some(contact) = selected else {
                    return Err(ContactResolverError::NoLiveContacts(aor_str.clone()));
                };

                Ok(ResolvedContact {
                    uri: contact.uri,
                    expires_at: Some(contact.expires),
                    source: ContactSource::Registrar,
                    transport: Some(format!("{:?}", contact.transport)),
                    received: contact.received,
                    path: contact.path,
                    instance_id: if contact.instance_id.is_empty() {
                        None
                    } else {
                        Some(contact.instance_id)
                    },
                    reg_id: contact.reg_id,
                    flow_id: None,
                    registered_flow_routes,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_registrar::{ContactInfo, ContactReachability, Transport};

    fn outbound_contact(
        token: String,
        reg_id: u32,
        port: u16,
        q_value: f32,
        expires: chrono::DateTime<Utc>,
    ) -> ContactInfo {
        ContactInfo {
            uri: format!("sip:alice@10.0.0.20:{port};transport=tls;ob"),
            instance_id: "urn:uuid:11111111-2222-4333-8444-555555555555".into(),
            transport: Transport::TLS,
            user_agent: "independent-test-ua/1.0".into(),
            expires,
            q_value,
            received: Some(format!("198.51.100.20:{port}")),
            path: Vec::new(),
            methods: vec!["INVITE".into()],
            reg_id: Some(reg_id),
            flow_id: Some(token),
            reachability: ContactReachability::Reachable,
        }
    }

    #[tokio::test]
    async fn registrar_resolution_returns_ordered_exact_flows_and_skips_degraded_primary() {
        let registrar = Arc::new(RegistrarService::new().await.unwrap());
        let aor = AddressOfRecord::parse("sip:alice@example.test").unwrap();
        let mut tokens = Vec::new();
        let expires = Utc::now() + chrono::Duration::minutes(10);
        for (reg_id, flow_id, port, q_value) in [(1, 11, 41001, 1.0), (2, 22, 41002, 0.9)] {
            let token = registrar.new_registered_flow_token();
            let contact = outbound_contact(token.clone(), reg_id, port, q_value, expires);
            registrar
                .bind_registered_flow(&aor, &contact, flow_id)
                .unwrap();
            registrar
                .register_aor(&aor, contact, Some(600))
                .await
                .unwrap();
            tokens.push(token);
        }

        let resolver = RegistrarContactResolver::new(Arc::clone(&registrar));
        let request = ContactRequest::Registered {
            aor: aor.as_str().to_string(),
        };
        let resolved = resolver.resolve_contact(&request).await.unwrap();
        assert_eq!(
            resolved
                .registered_flow_routes
                .iter()
                .map(RegisteredFlowRoute::process_local_flow_id)
                .collect::<Vec<_>>(),
            vec![11, 22]
        );
        let serialized = serde_json::to_string(&resolved).unwrap();
        assert!(!serialized.contains("rf1_"));
        assert!(!serialized.contains(&tokens[0]));

        registrar.mark_process_local_flow_unreachable(11).await;
        let resolved = resolver.resolve_contact(&request).await.unwrap();
        assert_eq!(resolved.registered_flow_routes.len(), 1);
        assert_eq!(
            resolved.registered_flow_routes[0].process_local_flow_id(),
            22
        );
        registrar.shutdown().await.unwrap();
    }
}
