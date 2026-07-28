use serde::{Deserialize, Deserializer, Serialize, Serializer};

const MAX_COMPONENT_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NamespaceComponent {
    Tenant,
    Broadcast,
}

impl std::fmt::Display for NamespaceComponent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tenant => formatter.write_str("tenant"),
            Self::Broadcast => formatter.write_str("broadcast"),
        }
    }
}

/// Validated `{tenant_id}/{broadcast_id}` namespace.
///
/// Components are not normalized or sanitized. Every accepted input therefore
/// has one exact wire representation; invalid values are rejected instead of
/// being mapped onto another tenant or broadcast's namespace.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MoqNamespace {
    tenant_id: String,
    broadcast_id: String,
    wire: String,
}

impl MoqNamespace {
    pub fn new(
        tenant_id: impl Into<String>,
        broadcast_id: impl Into<String>,
    ) -> Result<Self, MoqNamespaceError> {
        let tenant_id = tenant_id.into();
        let broadcast_id = broadcast_id.into();
        validate_component(NamespaceComponent::Tenant, &tenant_id)?;
        validate_component(NamespaceComponent::Broadcast, &broadcast_id)?;
        let wire = format!("{tenant_id}/{broadcast_id}");
        Ok(Self {
            tenant_id,
            broadcast_id,
            wire,
        })
    }

    pub fn parse(value: &str) -> Result<Self, MoqNamespaceError> {
        let mut components = value.split('/');
        let tenant = components.next().unwrap_or_default();
        let broadcast = components.next().ok_or(MoqNamespaceError::InvalidShape)?;
        if components.next().is_some() {
            return Err(MoqNamespaceError::InvalidShape);
        }
        Self::new(tenant, broadcast)
    }

    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    pub fn broadcast_id(&self) -> &str {
        &self.broadcast_id
    }

    pub fn as_str(&self) -> &str {
        &self.wire
    }
}

impl std::fmt::Display for MoqNamespace {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl AsRef<str> for MoqNamespace {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::str::FromStr for MoqNamespace {
    type Err = MoqNamespaceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for MoqNamespace {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for MoqNamespace {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MoqNamespaceError {
    #[error("MOQT namespace must be exactly tenant_id/broadcast_id")]
    InvalidShape,
    #[error("MOQT {component} namespace component is empty")]
    Empty { component: NamespaceComponent },
    #[error("MOQT {component} namespace component exceeds {maximum} bytes")]
    TooLong {
        component: NamespaceComponent,
        maximum: usize,
    },
    #[error("MOQT {component} namespace component is reserved: {value}")]
    Reserved {
        component: NamespaceComponent,
        value: String,
    },
    #[error(
        "MOQT {component} namespace component contains invalid character {character:?} at byte {index}"
    )]
    InvalidCharacter {
        component: NamespaceComponent,
        character: char,
        index: usize,
    },
}

fn validate_component(component: NamespaceComponent, value: &str) -> Result<(), MoqNamespaceError> {
    if value.is_empty() {
        return Err(MoqNamespaceError::Empty { component });
    }
    if value.len() > MAX_COMPONENT_BYTES {
        return Err(MoqNamespaceError::TooLong {
            component,
            maximum: MAX_COMPONENT_BYTES,
        });
    }
    if matches!(value, "." | "..") {
        return Err(MoqNamespaceError::Reserved {
            component,
            value: value.to_owned(),
        });
    }
    if component == NamespaceComponent::Tenant && value.starts_with('.') {
        return Err(MoqNamespaceError::Reserved {
            component,
            value: value.to_owned(),
        });
    }
    for (index, character) in value.char_indices() {
        if !(character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '~')) {
            return Err(MoqNamespaceError::InvalidCharacter {
                component,
                character,
                index,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_components_have_an_exact_collision_free_mapping() {
        let left = MoqNamespace::new("tenant-a", "broadcast_1").unwrap();
        let right = MoqNamespace::new("tenant_a", "broadcast-1").unwrap();
        assert_eq!(left.as_str(), "tenant-a/broadcast_1");
        assert_eq!(right.as_str(), "tenant_a/broadcast-1");
        assert_ne!(left, right);
        assert_eq!(left.to_string().parse::<MoqNamespace>().unwrap(), left);
    }

    #[test]
    fn values_that_used_to_collapse_during_sanitization_are_rejected() {
        for tenant in ["tenant/a", "tenant a", "tenant%2Fa", "ténant"] {
            assert!(MoqNamespace::new(tenant, "broadcast").is_err());
        }
        assert!(MoqNamespace::new("tenant", "../broadcast").is_err());
        assert!(MoqNamespace::new(".", "broadcast").is_err());
        assert!(MoqNamespace::new(".session", "broadcast").is_err());
        assert!(MoqNamespace::new(".private", "broadcast").is_err());
    }

    #[test]
    fn serde_cannot_bypass_validation() {
        let namespace = MoqNamespace::new("tenant", "broadcast").unwrap();
        assert_eq!(
            serde_json::to_string(&namespace).unwrap(),
            "\"tenant/broadcast\""
        );
        assert_eq!(
            serde_json::from_str::<MoqNamespace>("\"tenant/broadcast\"").unwrap(),
            namespace
        );
        assert!(serde_json::from_str::<MoqNamespace>("\"tenant/a/broadcast\"").is_err());
    }
}
