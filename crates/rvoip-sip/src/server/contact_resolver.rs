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
use rvoip_sip_registrar::{AddressOfRecord, RegistrarService};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;

/// Input to a [`ContactResolver`]. Either a literal SIP URI to dial, or an
/// AOR to look up against a registrar.
#[derive(Clone, Debug)]
pub enum ContactRequest {
    /// Use a literal SIP URI as-is (no registrar lookup).
    Static { uri: String },
    /// Look up a registered SIP user by AOR; the resolver consults the
    /// configured [`rvoip_sip_registrar::RegistrarService`] to find the live
    /// Contact binding.
    Registered { aor: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedContact {
    pub uri: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub source: ContactSource,
    pub transport: Option<String>,
    pub received: Option<String>,
    pub path: Vec<String>,
    pub instance_id: Option<String>,
    pub reg_id: Option<u32>,
    pub flow_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactSource {
    Static,
    Registrar,
    Custom,
}

#[derive(Debug, Error)]
pub enum ContactResolverError {
    #[error("contact resolution failed: {0}")]
    Failed(String),
    #[error("registrar error: {0}")]
    Registrar(String),
    #[error("invalid AOR `{aor}`: {detail}")]
    InvalidAor { aor: String, detail: String },
    #[error("AOR `{0}` has no live contacts")]
    NoLiveContacts(String),
}

#[async_trait]
pub trait ContactResolver: Send + Sync {
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

                let Some(contact) = contacts.into_iter().next() else {
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
                    flow_id: contact.flow_id,
                })
            }
        }
    }
}
