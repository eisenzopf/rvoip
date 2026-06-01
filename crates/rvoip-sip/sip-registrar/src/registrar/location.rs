//! Location service for mapping addresses-of-record to contact bindings.

use crate::error::{RegistrarError, Result};
use crate::registrar::registry::UserRegistry;
use crate::types::{AddressOfRecord, ContactInfo};
use std::sync::Arc;

/// Read/query facade over the authoritative registration binding store.
pub struct LocationService {
    registry: Arc<UserRegistry>,
}

impl LocationService {
    pub fn new(registry: Arc<UserRegistry>) -> Self {
        Self { registry }
    }

    /// Compatibility method. Writes are owned by `UserRegistry`.
    pub async fn add_binding(&self, user_id: &str, contact: ContactInfo) -> Result<()> {
        self.registry.register(user_id, contact, 3600).await
    }

    /// Compatibility method. Writes are owned by `UserRegistry`.
    pub async fn remove_binding(&self, user_id: &str, contact_uri: &str) -> Result<()> {
        self.registry.remove_contact(user_id, contact_uri).await
    }

    pub async fn find_contacts(&self, user_id: &str) -> Result<Vec<ContactInfo>> {
        self.registry.lookup_contacts(user_id).await
    }

    pub async fn find_aor_contacts(&self, aor: &AddressOfRecord) -> Result<Vec<ContactInfo>> {
        self.registry.lookup_contacts(aor.as_str()).await
    }

    pub async fn find_live_contacts(
        &self,
        aor: &AddressOfRecord,
        method: &str,
    ) -> Result<Vec<ContactInfo>> {
        self.registry
            .lookup_live_contacts(aor.as_str(), method)
            .await
    }

    pub async fn find_user(&self, contact_uri: &str) -> Option<String> {
        for registration in self.registry.get_all_registrations().await {
            if registration
                .contacts
                .iter()
                .any(|contact| contact.uri == contact_uri)
            {
                return registration
                    .aor
                    .map(|aor| aor.to_string())
                    .or(Some(registration.user_id));
            }
        }
        None
    }

    pub async fn clear_user(&self, user_id: &str) -> Result<()> {
        match self.registry.unregister(user_id).await {
            Ok(()) | Err(RegistrarError::UserNotFound(_)) => Ok(()),
            Err(error) => Err(error),
        }
    }
}
