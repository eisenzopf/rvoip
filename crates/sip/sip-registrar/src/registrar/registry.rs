//! User registry for managing registrations

use crate::error::{RegistrarError, Result};
use crate::types::{
    AddressOfRecord, ContactInfo, ContactReachability, RegistrarConfig, UserRegistration,
};
use chrono::{DateTime, Duration, Utc};
use dashmap::DashMap;
use std::cmp::Ordering;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Thread-safe authoritative registration binding store.
pub struct UserRegistry {
    users: Arc<DashMap<String, UserRegistration>>,
    config: RegistryConfig,
}

#[derive(Debug, Clone)]
pub struct RegistryConfig {
    pub max_contacts_per_aor: usize,
    pub remove_existing: bool,
    pub remove_unavailable: bool,
    pub support_path: bool,
    pub default_expires: u32,
    pub max_expires: u32,
    pub min_expires: u32,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            max_contacts_per_aor: 10,
            remove_existing: false,
            remove_unavailable: true,
            support_path: true,
            default_expires: 3600,
            max_expires: 86400,
            min_expires: 60,
        }
    }
}

impl From<&RegistrarConfig> for RegistryConfig {
    fn from(config: &RegistrarConfig) -> Self {
        Self {
            max_contacts_per_aor: config.max_contacts_per_aor,
            remove_existing: config.remove_existing,
            remove_unavailable: config.remove_unavailable,
            support_path: config.support_path,
            default_expires: config.default_expires,
            max_expires: config.max_expires,
            min_expires: config.min_expires,
        }
    }
}

impl UserRegistry {
    pub fn new() -> Self {
        Self::with_config(RegistryConfig::default())
    }

    pub fn with_config(config: RegistryConfig) -> Self {
        Self {
            users: Arc::new(DashMap::new()),
            config,
        }
    }

    pub async fn register(&self, key: &str, contact: ContactInfo, expires: u32) -> Result<()> {
        self.register_binding(key, None, contact, expires).await
    }

    pub async fn register_aor(
        &self,
        aor: &AddressOfRecord,
        contact: ContactInfo,
        expires: u32,
    ) -> Result<()> {
        self.register_binding(aor.as_str(), Some(aor.clone()), contact, expires)
            .await
    }

    pub async fn register_contacts(
        &self,
        aor: &AddressOfRecord,
        contacts: Vec<ContactInfo>,
        expires: u32,
    ) -> Result<()> {
        let expires = self.validate_expires(expires)?;
        if expires == 0 {
            for contact in contacts {
                self.remove_contact(aor.as_str(), &contact.uri).await?;
            }
            return Ok(());
        }

        let expires_at = Utc::now() + Duration::seconds(expires as i64);
        let mut next = self
            .users
            .get(aor.as_str())
            .map(|entry| entry.clone())
            .unwrap_or_else(|| UserRegistration {
                user_id: aor.user().to_string(),
                aor: Some(aor.clone()),
                contacts: Vec::new(),
                expires: expires_at,
                presence_enabled: true,
                capabilities: vec!["presence".to_string()],
                registered_at: Utc::now(),
                attributes: Default::default(),
            });

        for mut contact in contacts {
            if !self.config.support_path {
                contact.path.clear();
            }
            contact.expires = expires_at;
            self.update_contact(&mut next, contact, expires_at)?;
        }

        self.users.insert(aor.to_string(), next);
        Ok(())
    }

    async fn register_binding(
        &self,
        key: &str,
        aor: Option<AddressOfRecord>,
        mut contact: ContactInfo,
        expires: u32,
    ) -> Result<()> {
        let expires = self.validate_expires(expires)?;
        if expires == 0 {
            return self.remove_contact(key, &contact.uri).await;
        }
        if !self.config.support_path {
            contact.path.clear();
        }

        let expires_at = Utc::now() + Duration::seconds(expires as i64);
        contact.expires = expires_at;
        let contact_present = !contact.uri.is_empty();
        let contact_bytes = contact.uri.len();
        let user_id = aor
            .as_ref()
            .map(|aor| aor.user().to_string())
            .unwrap_or_else(|| key.to_string());

        if let Some(mut registration) = self.users.get_mut(key) {
            self.update_contact(&mut registration, contact, expires_at)?;
        } else {
            let mut registration = UserRegistration {
                user_id,
                aor,
                contacts: Vec::new(),
                expires: expires_at,
                presence_enabled: true,
                capabilities: vec!["presence".to_string()],
                registered_at: Utc::now(),
                attributes: Default::default(),
            };
            self.update_contact(&mut registration, contact, expires_at)?;
            self.users.insert(key.to_string(), registration);
        }

        info!(
            stage = "binding-update",
            operation = "register",
            identity_present = !key.is_empty(),
            identity_bytes = key.len(),
            contact_present,
            contact_bytes,
            "Registration binding updated"
        );
        Ok(())
    }

    pub async fn unregister(&self, key: &str) -> Result<()> {
        if self.users.remove(key).is_some() {
            info!(
                stage = "binding-update",
                operation = "unregister",
                identity_present = !key.is_empty(),
                identity_bytes = key.len(),
                "Registration binding removed"
            );
            Ok(())
        } else {
            Err(RegistrarError::UserNotFound(key.to_string()))
        }
    }

    pub async fn remove_contact(&self, key: &str, contact_uri: &str) -> Result<()> {
        let mut entry = self
            .users
            .get_mut(key)
            .ok_or_else(|| RegistrarError::UserNotFound(key.to_string()))?;

        let initial_count = entry.contacts.len();
        entry.contacts.retain(|c| c.uri != contact_uri);

        if entry.contacts.len() == initial_count {
            return Err(RegistrarError::ContactNotFound {
                user: key.to_string(),
                uri: contact_uri.to_string(),
            });
        }

        if entry.contacts.is_empty() {
            drop(entry);
            self.users.remove(key);
            info!(
                stage = "binding-update",
                operation = "remove-last-contact",
                identity_present = !key.is_empty(),
                identity_bytes = key.len(),
                "Registration binding removed"
            );
        } else {
            entry.expires = latest_expiry(&entry.contacts);
        }

        Ok(())
    }

    pub async fn clear_bindings(&self, key: &str) -> Result<()> {
        self.unregister(key).await
    }

    pub async fn refresh(&self, key: &str, contact_uri: &str, expires: u32) -> Result<()> {
        let expires = self.validate_expires(expires)?;
        if expires == 0 {
            return self.remove_contact(key, contact_uri).await;
        }

        let expires_at = Utc::now() + Duration::seconds(expires as i64);
        let mut entry = self
            .users
            .get_mut(key)
            .ok_or_else(|| RegistrarError::UserNotFound(key.to_string()))?;

        let contact = entry
            .contacts
            .iter_mut()
            .find(|c| c.uri == contact_uri)
            .ok_or_else(|| RegistrarError::ContactNotFound {
                user: key.to_string(),
                uri: contact_uri.to_string(),
            })?;

        contact.expires = expires_at;
        entry.expires = latest_expiry(&entry.contacts);

        debug!(
            stage = "binding-refresh",
            identity_present = !key.is_empty(),
            identity_bytes = key.len(),
            contact_present = !contact_uri.is_empty(),
            contact_bytes = contact_uri.len(),
            "Registration binding refreshed"
        );
        Ok(())
    }

    pub async fn get_registration(&self, key: &str) -> Result<UserRegistration> {
        self.users
            .get(key)
            .map(|entry| entry.clone())
            .ok_or_else(|| RegistrarError::UserNotFound(key.to_string()))
    }

    pub async fn lookup_contacts(&self, key: &str) -> Result<Vec<ContactInfo>> {
        Ok(self.get_registration(key).await?.contacts)
    }

    pub async fn lookup_live_contacts(&self, key: &str, method: &str) -> Result<Vec<ContactInfo>> {
        let now = Utc::now();
        let mut contacts: Vec<_> = self
            .lookup_contacts(key)
            .await?
            .into_iter()
            .filter(|contact| contact.is_live_for(method, now))
            .collect();
        contacts.sort_by(contact_preference);
        Ok(contacts)
    }

    pub async fn set_reachability(
        &self,
        key: &str,
        contact_uri: &str,
        reachability: ContactReachability,
    ) -> Result<()> {
        let mut entry = self
            .users
            .get_mut(key)
            .ok_or_else(|| RegistrarError::UserNotFound(key.to_string()))?;
        let contact = entry
            .contacts
            .iter_mut()
            .find(|contact| contact.uri == contact_uri)
            .ok_or_else(|| RegistrarError::ContactNotFound {
                user: key.to_string(),
                uri: contact_uri.to_string(),
            })?;
        contact.reachability = reachability;
        Ok(())
    }

    pub async fn is_registered(&self, key: &str) -> bool {
        self.users.contains_key(key)
    }

    pub async fn list_all_users(&self) -> Vec<String> {
        self.users.iter().map(|entry| entry.key().clone()).collect()
    }

    pub async fn get_all_registrations(&self) -> Vec<UserRegistration> {
        self.users
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    pub async fn expire_registrations(&self) -> Vec<String> {
        let now = Utc::now();
        let mut expired_users = Vec::new();
        let keys: Vec<String> = self.users.iter().map(|entry| entry.key().clone()).collect();

        for key in keys {
            let mut should_remove = false;
            if let Some(mut entry) = self.users.get_mut(&key) {
                entry.contacts.retain(|contact| contact.expires > now);
                if entry.contacts.is_empty() {
                    should_remove = true;
                } else {
                    entry.expires = latest_expiry(&entry.contacts);
                }
            }

            if should_remove && self.users.remove(&key).is_some() {
                warn!(
                    stage = "binding-expiry",
                    identity_present = !key.is_empty(),
                    identity_bytes = key.len(),
                    "Registration binding expired"
                );
                expired_users.push(key);
            }
        }

        expired_users
    }

    fn update_contact(
        &self,
        registration: &mut UserRegistration,
        contact: ContactInfo,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        registration
            .contacts
            .retain(|existing| !same_binding(existing, &contact));

        self.apply_contact_limit(registration)?;
        registration.contacts.push(contact);
        registration.expires = registration.expires.max(expires_at);
        Ok(())
    }

    fn apply_contact_limit(&self, registration: &mut UserRegistration) -> Result<()> {
        if self.config.max_contacts_per_aor == 0 {
            return Err(RegistrarError::MaxContactsExceeded {
                user: registration
                    .aor
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| registration.user_id.clone()),
                max: self.config.max_contacts_per_aor,
            });
        }

        if registration.contacts.len() < self.config.max_contacts_per_aor {
            return Ok(());
        }

        if self.config.remove_unavailable {
            registration
                .contacts
                .retain(|contact| contact.reachability != ContactReachability::Unreachable);
            if registration.contacts.len() < self.config.max_contacts_per_aor {
                return Ok(());
            }
        }

        if self.config.remove_existing {
            registration.contacts.sort_by(lowest_preference_first);
            if !registration.contacts.is_empty() {
                registration.contacts.remove(0);
            }
            return Ok(());
        }

        Err(RegistrarError::MaxContactsExceeded {
            user: registration
                .aor
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| registration.user_id.clone()),
            max: self.config.max_contacts_per_aor,
        })
    }

    fn validate_expires(&self, expires: u32) -> Result<u32> {
        if expires == 0 {
            return Ok(0);
        }

        if expires < self.config.min_expires {
            return Ok(self.config.min_expires);
        }

        if expires > self.config.max_expires {
            return Ok(self.config.max_expires);
        }

        Ok(expires)
    }
}

fn same_binding(existing: &ContactInfo, incoming: &ContactInfo) -> bool {
    existing.uri == incoming.uri
        || (incoming.reg_id.is_some()
            && !incoming.instance_id.is_empty()
            && existing.instance_id == incoming.instance_id
            && existing.reg_id == incoming.reg_id)
}

fn latest_expiry(contacts: &[ContactInfo]) -> DateTime<Utc> {
    contacts
        .iter()
        .map(|contact| contact.expires)
        .max()
        .unwrap_or_else(Utc::now)
}

fn contact_preference(a: &ContactInfo, b: &ContactInfo) -> Ordering {
    reachability_rank(b)
        .cmp(&reachability_rank(a))
        .then_with(|| b.q_value.partial_cmp(&a.q_value).unwrap_or(Ordering::Equal))
        .then_with(|| b.expires.cmp(&a.expires))
        .then_with(|| a.uri.cmp(&b.uri))
}

fn lowest_preference_first(a: &ContactInfo, b: &ContactInfo) -> Ordering {
    reachability_rank(a)
        .cmp(&reachability_rank(b))
        .then_with(|| a.q_value.partial_cmp(&b.q_value).unwrap_or(Ordering::Equal))
        .then_with(|| a.expires.cmp(&b.expires))
        .then_with(|| a.uri.cmp(&b.uri))
}

fn reachability_rank(contact: &ContactInfo) -> u8 {
    match contact.reachability {
        ContactReachability::Reachable => 2,
        ContactReachability::Unknown => 1,
        ContactReachability::Unreachable => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Transport;

    fn contact(uri: &str, q_value: f32) -> ContactInfo {
        ContactInfo {
            uri: uri.to_string(),
            instance_id: "device-1".to_string(),
            transport: Transport::UDP,
            user_agent: "Test UA".to_string(),
            expires: Utc::now() + Duration::hours(1),
            q_value,
            received: None,
            path: vec![],
            methods: vec!["INVITE".to_string(), "MESSAGE".to_string()],
            reg_id: None,
            flow_id: None,
            reachability: ContactReachability::Unknown,
        }
    }

    #[test]
    fn registry_logs_only_structural_binding_metadata() {
        let source = include_str!("registry.rs");

        for fragments in [
            ["Registration {} updated", " with contact {}"],
            ["Registration {}", " removed"],
            ["Refreshed registration", " for {}:{}"],
            ["Registration expired", " for {}"],
        ] {
            let forbidden = fragments.concat();
            assert!(
                !source.contains(&forbidden),
                "registry regained value-bearing diagnostic: {forbidden}"
            );
        }

        for required in [
            "stage = \"binding-update\"",
            "stage = \"binding-refresh\"",
            "stage = \"binding-expiry\"",
            "identity_present",
            "identity_bytes",
            "contact_present",
            "contact_bytes",
        ] {
            assert!(
                source.contains(required),
                "registry diagnostic lost structural field: {required}"
            );
        }
    }

    #[tokio::test]
    async fn test_user_registration() {
        let registry = UserRegistry::new();
        let contact = contact("sip:alice@192.168.1.100:5060", 1.0);

        registry
            .register("alice", contact.clone(), 3600)
            .await
            .unwrap();

        assert!(registry.is_registered("alice").await);

        let reg = registry.get_registration("alice").await.unwrap();
        assert_eq!(reg.user_id, "alice");
        assert_eq!(reg.contacts.len(), 1);
        assert_eq!(reg.contacts[0].uri, contact.uri);
    }

    #[tokio::test]
    async fn test_unregister() {
        let registry = UserRegistry::new();
        registry
            .register("bob", contact("sip:bob@example.com", 1.0), 3600)
            .await
            .unwrap();
        assert!(registry.is_registered("bob").await);

        registry.unregister("bob").await.unwrap();
        assert!(!registry.is_registered("bob").await);
    }
}
