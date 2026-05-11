//! User registration and location services

use crate::error::{RegistrarError, Result};
use crate::types::{
    AddressOfRecord, ContactInfo, ContactReachability, RegistrarConfig, UserRegistration,
};
use dashmap::DashMap;
use std::sync::Arc;

pub mod location;
pub mod manager;
pub mod registry;
pub mod user_store;

pub use location::LocationService;
pub use manager::RegistrationManager;
pub use registry::{RegistryConfig, UserRegistry};
pub use user_store::{UserCredentials, UserStore};

/// Main registrar interface combining registration, lookup, and expiry.
pub struct Registrar {
    registry: Arc<UserRegistry>,
    location: Arc<LocationService>,
    manager: Arc<RegistrationManager>,
    domain_aliases: Arc<DashMap<String, String>>,
}

impl Registrar {
    pub fn new() -> Self {
        Self::with_config(RegistrarConfig::default())
    }

    pub fn with_config(config: RegistrarConfig) -> Self {
        let registry = Arc::new(UserRegistry::with_config(RegistryConfig::from(&config)));
        let location = Arc::new(LocationService::new(registry.clone()));
        let manager = Arc::new(RegistrationManager::new(registry.clone()));

        Self {
            registry,
            location,
            manager,
            domain_aliases: Arc::new(DashMap::new()),
        }
    }

    pub fn add_domain_alias(&self, alias: impl Into<String>, target: impl Into<String>) {
        self.domain_aliases.insert(
            alias.into().to_ascii_lowercase(),
            target.into().to_ascii_lowercase(),
        );
    }

    pub fn canonicalize_aor(&self, aor: &AddressOfRecord) -> Result<AddressOfRecord> {
        if let Some(target) = self.domain_aliases.get(aor.domain()) {
            aor.with_domain(target.value())
                .map_err(RegistrarError::InvalidRegistration)
        } else {
            Ok(aor.clone())
        }
    }

    pub async fn register_aor(
        &self,
        aor: &AddressOfRecord,
        contact: ContactInfo,
        expires: u32,
    ) -> Result<()> {
        let aor = self.canonicalize_aor(aor)?;
        self.registry.register_aor(&aor, contact, expires).await
    }

    pub async fn register_contacts(
        &self,
        aor: &AddressOfRecord,
        contacts: Vec<ContactInfo>,
        expires: u32,
    ) -> Result<()> {
        let aor = self.canonicalize_aor(aor)?;
        self.registry
            .register_contacts(&aor, contacts, expires)
            .await
    }

    pub async fn unregister_aor(&self, aor: &AddressOfRecord) -> Result<()> {
        let aor = self.canonicalize_aor(aor)?;
        self.registry.unregister(aor.as_str()).await
    }

    pub async fn unregister_all_bindings(&self, aor: &AddressOfRecord) -> Result<()> {
        self.unregister_aor(aor).await
    }

    pub async fn unregister_contact_aor(
        &self,
        aor: &AddressOfRecord,
        contact_uri: &str,
    ) -> Result<()> {
        let aor = self.canonicalize_aor(aor)?;
        self.registry
            .remove_contact(aor.as_str(), contact_uri)
            .await
    }

    pub async fn refresh_registration_aor(
        &self,
        aor: &AddressOfRecord,
        contact_uri: &str,
        expires: u32,
    ) -> Result<()> {
        let aor = self.canonicalize_aor(aor)?;
        self.registry
            .refresh(aor.as_str(), contact_uri, expires)
            .await
    }

    pub async fn lookup_aor(&self, aor: &AddressOfRecord) -> Result<Vec<ContactInfo>> {
        let aor = self.canonicalize_aor(aor)?;
        self.location.find_aor_contacts(&aor).await
    }

    pub async fn lookup_live_contacts(
        &self,
        aor: &AddressOfRecord,
        method: &str,
    ) -> Result<Vec<ContactInfo>> {
        let aor = self.canonicalize_aor(aor)?;
        self.location.find_live_contacts(&aor, method).await
    }

    pub async fn set_contact_reachability(
        &self,
        aor: &AddressOfRecord,
        contact_uri: &str,
        reachability: ContactReachability,
    ) -> Result<()> {
        let aor = self.canonicalize_aor(aor)?;
        self.registry
            .set_reachability(aor.as_str(), contact_uri, reachability)
            .await
    }

    /// Compatibility registration using the legacy user id key.
    pub async fn register_user(
        &self,
        user_id: &str,
        contact: ContactInfo,
        expires: u32,
    ) -> Result<()> {
        self.registry.register(user_id, contact, expires).await
    }

    pub async fn unregister_user(&self, user_id: &str) -> Result<()> {
        self.registry.unregister(user_id).await
    }

    pub async fn unregister_contact(&self, user_id: &str, contact_uri: &str) -> Result<()> {
        self.registry.remove_contact(user_id, contact_uri).await
    }

    pub async fn lookup_user(&self, user_id: &str) -> Result<Vec<ContactInfo>> {
        self.location.find_contacts(user_id).await
    }

    pub async fn refresh_registration(
        &self,
        user_id: &str,
        contact_uri: &str,
        expires: u32,
    ) -> Result<()> {
        self.registry.refresh(user_id, contact_uri, expires).await
    }

    pub async fn list_users(&self) -> Vec<String> {
        self.registry.list_all_users().await
    }

    pub async fn is_registered(&self, user_id: &str) -> bool {
        self.registry.is_registered(user_id).await
    }

    pub async fn get_registration(&self, user_id: &str) -> Result<UserRegistration> {
        self.registry.get_registration(user_id).await
    }

    pub async fn start_expiry_manager(&self) {
        self.manager.start().await
    }

    pub async fn stop_expiry_manager(&self) {
        self.manager.stop().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Transport;
    use chrono::{Duration, Utc};

    fn contact(uri: &str) -> ContactInfo {
        ContactInfo {
            uri: uri.to_string(),
            instance_id: format!("instance-{uri}"),
            transport: Transport::UDP,
            user_agent: "registrar-core-test".to_string(),
            expires: Utc::now() + Duration::minutes(5),
            q_value: 1.0,
            received: None,
            path: Vec::new(),
            methods: vec!["INVITE".to_string()],
            reg_id: None,
            flow_id: None,
            reachability: ContactReachability::Unknown,
        }
    }

    #[tokio::test]
    async fn register_user_populates_location_lookup() {
        let registrar = Registrar::new();
        registrar
            .register_user("alice", contact("sip:alice@127.0.0.1:5071"), 300)
            .await
            .unwrap();

        let contacts = registrar.lookup_user("alice").await.unwrap();
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].uri, "sip:alice@127.0.0.1:5071");
    }

    #[tokio::test]
    async fn unregister_contact_removes_location_binding() {
        let registrar = Registrar::new();
        registrar
            .register_user("alice", contact("sip:alice@127.0.0.1:5071"), 300)
            .await
            .unwrap();
        registrar
            .unregister_contact("alice", "sip:alice@127.0.0.1:5071")
            .await
            .unwrap();

        assert!(registrar.lookup_user("alice").await.is_err());
    }
}
