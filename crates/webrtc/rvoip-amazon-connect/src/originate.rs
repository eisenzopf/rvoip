//! Typed, bounded options for one generic Amazon Connect origination.
//!
//! The legacy screen-pop wrappers intentionally continue to accept
//! [`crate::ContactTarget`] and to omit `ClientToken`. New orchestrated calls
//! must use the exact types in this module so profile selection, target data,
//! attributes, and idempotency remain immutable across prepare/reconcile.

use std::collections::BTreeMap;
use std::fmt;

use thiserror::Error;
use zeroize::Zeroize;

use crate::control::StartContactRequest;
use crate::mapping::MAX_ATTRIBUTE_BYTES;

/// Profile installed by [`crate::AmazonConnectAdapter::new`].
pub const DEFAULT_CONNECT_PROFILE_ID: &str = "default";

/// Maximum UTF-8 bytes in a non-secret configured profile identifier.
pub const MAX_CONNECT_PROFILE_ID_BYTES: usize = 128;
/// Maximum UTF-8 bytes in an instance or contact-flow identifier/ARN.
pub const MAX_CONNECT_RESOURCE_ID_BYTES: usize = 512;
/// Maximum UTF-8 bytes in the participant display name.
pub const MAX_CONNECT_DISPLAY_NAME_BYTES: usize = 256;
/// Maximum UTF-8 bytes in the optional task description.
pub const MAX_CONNECT_DESCRIPTION_BYTES: usize = 4_096;
/// Maximum UTF-8 bytes in a caller-stable Connect idempotency token.
pub const MAX_CONNECT_CLIENT_TOKEN_BYTES: usize = 1_024;
/// Defensive cardinality bound in addition to Connect's aggregate byte bound.
pub const MAX_CONNECT_ATTRIBUTE_COUNT: usize = 256;
/// Defensive bound for one documented `[A-Za-z0-9_-]+` attribute key.
pub const MAX_CONNECT_ATTRIBUTE_KEY_BYTES: usize = 128;

/// Validation failure for Amazon-owned outbound options.
///
/// Variants intentionally contain no caller-supplied value.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum AmazonConnectOriginateContextError {
    /// Profile IDs are non-empty configured names using `[A-Za-z0-9._-]+`.
    #[error("the Amazon Connect profile identifier is invalid")]
    InvalidProfileId,
    /// The profile identifier exceeded its local admission bound.
    #[error("the Amazon Connect profile identifier is too large")]
    ProfileIdTooLarge,
    /// The instance identifier/ARN was empty or contained forbidden bytes.
    #[error("the Amazon Connect instance identifier is invalid")]
    InvalidInstanceId,
    /// The instance identifier/ARN exceeded its local admission bound.
    #[error("the Amazon Connect instance identifier is too large")]
    InstanceIdTooLarge,
    /// The contact-flow identifier/ARN was empty or contained forbidden bytes.
    #[error("the Amazon Connect contact-flow identifier is invalid")]
    InvalidContactFlowId,
    /// The contact-flow identifier/ARN exceeded its local admission bound.
    #[error("the Amazon Connect contact-flow identifier is too large")]
    ContactFlowIdTooLarge,
    /// The display name was empty or contained forbidden control bytes.
    #[error("the Amazon Connect display name is invalid")]
    InvalidDisplayName,
    /// The display name exceeded its local admission bound.
    #[error("the Amazon Connect display name is too large")]
    DisplayNameTooLarge,
    /// The optional description was empty or contained control bytes.
    #[error("the Amazon Connect description is invalid")]
    InvalidDescription,
    /// The optional description exceeded its local admission bound.
    #[error("the Amazon Connect description is too large")]
    DescriptionTooLarge,
    /// The stable idempotency token was empty or contained forbidden bytes.
    #[error("the Amazon Connect client token is invalid")]
    InvalidClientToken,
    /// The stable idempotency token exceeded its local admission bound.
    #[error("the Amazon Connect client token is too large")]
    ClientTokenTooLarge,
    /// The attribute map exceeded its defensive cardinality bound.
    #[error("too many Amazon Connect originate attributes")]
    TooManyAttributes,
    /// One attribute key was empty, too large, or outside Connect's charset.
    #[error("an Amazon Connect originate attribute key is invalid")]
    InvalidAttributeKey,
    /// One attribute value contained a control byte.
    #[error("an Amazon Connect originate attribute value is invalid")]
    InvalidAttributeValue,
    /// Connect's documented aggregate 32-KiB attribute budget was exceeded.
    #[error("Amazon Connect originate attributes are too large in aggregate")]
    AttributesTooLarge,
}

/// Non-secret identifier selecting one adapter-owned AWS profile/starter.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ConnectProfileId(String);

impl ConnectProfileId {
    /// Validate and retain a configured profile identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, AmazonConnectOriginateContextError> {
        let value = value.into();
        validate_profile_id(&value)?;
        Ok(Self(value))
    }

    /// The configured identifier. Do not use this value as a metric label.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ConnectProfileId {
    fn default() -> Self {
        Self(DEFAULT_CONNECT_PROFILE_ID.to_owned())
    }
}

impl fmt::Debug for ConnectProfileId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ConnectProfileId([redacted])")
    }
}

impl fmt::Display for ConnectProfileId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

/// Exact Amazon Connect instance and contact-flow target for one call.
#[derive(Clone)]
pub struct AmazonConnectTarget {
    instance_id: String,
    contact_flow_id: String,
}

impl AmazonConnectTarget {
    /// Validate an exact instance and contact-flow pair. No adapter defaults
    /// are consulted by the generic path.
    pub fn new(
        instance_id: impl Into<String>,
        contact_flow_id: impl Into<String>,
    ) -> Result<Self, AmazonConnectOriginateContextError> {
        let mut instance_id = instance_id.into();
        let mut contact_flow_id = contact_flow_id.into();
        if let Err(error) = validate_resource_id(
            &instance_id,
            AmazonConnectOriginateContextError::InvalidInstanceId,
            AmazonConnectOriginateContextError::InstanceIdTooLarge,
        ) {
            instance_id.zeroize();
            contact_flow_id.zeroize();
            return Err(error);
        }
        if let Err(error) = validate_resource_id(
            &contact_flow_id,
            AmazonConnectOriginateContextError::InvalidContactFlowId,
            AmazonConnectOriginateContextError::ContactFlowIdTooLarge,
        ) {
            instance_id.zeroize();
            contact_flow_id.zeroize();
            return Err(error);
        }
        Ok(Self {
            instance_id,
            contact_flow_id,
        })
    }

    /// Exact instance identifier/ARN.
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    /// Exact contact-flow identifier/ARN.
    pub fn contact_flow_id(&self) -> &str {
        &self.contact_flow_id
    }

    fn validate(&self) -> Result<(), AmazonConnectOriginateContextError> {
        validate_resource_id(
            &self.instance_id,
            AmazonConnectOriginateContextError::InvalidInstanceId,
            AmazonConnectOriginateContextError::InstanceIdTooLarge,
        )?;
        validate_resource_id(
            &self.contact_flow_id,
            AmazonConnectOriginateContextError::InvalidContactFlowId,
            AmazonConnectOriginateContextError::ContactFlowIdTooLarge,
        )
    }
}

impl fmt::Debug for AmazonConnectTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AmazonConnectTarget")
            .field("instance_id", &"[redacted]")
            .field("contact_flow_id", &"[redacted]")
            .finish()
    }
}

impl Drop for AmazonConnectTarget {
    fn drop(&mut self) {
        self.instance_id.zeroize();
        self.contact_flow_id.zeroize();
    }
}

/// Caller-stable, case-sensitive token used for Connect idempotency.
#[derive(Clone, Eq, PartialEq)]
pub struct ConnectClientToken(String);

impl ConnectClientToken {
    /// Validate and retain a stable token. The adapter never generates a
    /// replacement token while reconciling the same immutable request.
    pub fn new(value: impl Into<String>) -> Result<Self, AmazonConnectOriginateContextError> {
        let mut value = value.into();
        if value.len() > MAX_CONNECT_CLIENT_TOKEN_BYTES {
            value.zeroize();
            return Err(AmazonConnectOriginateContextError::ClientTokenTooLarge);
        }
        if value.is_empty()
            || !value.is_ascii()
            || value
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
        {
            value.zeroize();
            return Err(AmazonConnectOriginateContextError::InvalidClientToken);
        }
        Ok(Self(value))
    }

    /// Explicit access for the AWS request builder. Diagnostics remain redacted.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ConnectClientToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ConnectClientToken([redacted])")
    }
}

impl fmt::Display for ConnectClientToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

impl Drop for ConnectClientToken {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Immutable Amazon-specific context carried opaquely by `OriginateRequest`.
#[derive(Clone)]
pub struct AmazonConnectOriginateContext {
    profile_id: ConnectProfileId,
    target: AmazonConnectTarget,
    attributes: BTreeMap<String, String>,
    display_name: String,
    description: Option<String>,
    client_token: ConnectClientToken,
}

impl AmazonConnectOriginateContext {
    /// Construct an exact, fully bounded context for a generic Amazon call.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        profile_id: ConnectProfileId,
        target: AmazonConnectTarget,
        attributes: BTreeMap<String, String>,
        display_name: impl Into<String>,
        description: Option<String>,
        client_token: ConnectClientToken,
    ) -> Result<Self, AmazonConnectOriginateContextError> {
        let mut context = Self {
            profile_id,
            target,
            attributes,
            display_name: display_name.into(),
            description,
            client_token,
        };
        if let Err(error) = context.validate() {
            context.zeroize_retained_values();
            return Err(error);
        }
        Ok(context)
    }

    /// Selected adapter-owned profile.
    pub fn profile_id(&self) -> &ConnectProfileId {
        &self.profile_id
    }

    /// Exact Connect target.
    pub fn target(&self) -> &AmazonConnectTarget {
        &self.target
    }

    /// Exact contact attributes. Treat values as sensitive customer context.
    pub fn attributes(&self) -> &BTreeMap<String, String> {
        &self.attributes
    }

    /// Participant display name. Treat it as customer context.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Optional CCP task description.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Stable idempotency token.
    pub fn client_token(&self) -> &ConnectClientToken {
        &self.client_token
    }

    /// Revalidate all retained fields at the opaque adapter boundary.
    pub fn validate(&self) -> Result<(), AmazonConnectOriginateContextError> {
        validate_profile_id(self.profile_id.as_str())?;
        self.target.validate()?;
        validate_display_name(&self.display_name)?;
        validate_description(self.description.as_deref())?;
        validate_client_token(self.client_token.expose_secret())?;
        validate_attributes(&self.attributes)
    }

    /// Produce the byte-for-byte equivalent application request used by
    /// activation and reconciliation. Repeated calls preserve every field and
    /// the same stable token; no default or random token is introduced.
    pub fn start_request(&self) -> StartContactRequest {
        StartContactRequest {
            instance_id: self.target.instance_id.clone(),
            contact_flow_id: self.target.contact_flow_id.clone(),
            display_name: self.display_name.clone(),
            attributes: self.attributes.clone(),
            description: self.description.clone(),
            client_token: Some(self.client_token.0.clone()),
        }
    }

    fn zeroize_retained_values(&mut self) {
        self.display_name.zeroize();
        if let Some(description) = self.description.as_mut() {
            description.zeroize();
        }
        for (mut key, mut value) in std::mem::take(&mut self.attributes) {
            key.zeroize();
            value.zeroize();
        }
    }
}

impl fmt::Debug for AmazonConnectOriginateContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AmazonConnectOriginateContext")
            .field("profile_id", &self.profile_id)
            .field("target", &self.target)
            .field("attribute_count", &self.attributes.len())
            .field("display_name_present", &!self.display_name.is_empty())
            .field("description_present", &self.description.is_some())
            .field("client_token", &self.client_token)
            .finish()
    }
}

impl Drop for AmazonConnectOriginateContext {
    fn drop(&mut self) {
        self.zeroize_retained_values();
    }
}

pub(crate) fn validate_start_contact_request(
    request: &StartContactRequest,
) -> Result<(), AmazonConnectOriginateContextError> {
    validate_resource_id(
        &request.instance_id,
        AmazonConnectOriginateContextError::InvalidInstanceId,
        AmazonConnectOriginateContextError::InstanceIdTooLarge,
    )?;
    validate_resource_id(
        &request.contact_flow_id,
        AmazonConnectOriginateContextError::InvalidContactFlowId,
        AmazonConnectOriginateContextError::ContactFlowIdTooLarge,
    )?;
    validate_display_name(&request.display_name)?;
    validate_description(request.description.as_deref())?;
    match request.client_token.as_deref() {
        Some(token) => validate_client_token(token)?,
        None => return Err(AmazonConnectOriginateContextError::InvalidClientToken),
    }
    validate_attributes(&request.attributes)
}

fn validate_profile_id(value: &str) -> Result<(), AmazonConnectOriginateContextError> {
    if value.len() > MAX_CONNECT_PROFILE_ID_BYTES {
        return Err(AmazonConnectOriginateContextError::ProfileIdTooLarge);
    }
    if value.is_empty()
        || !value.is_ascii()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(AmazonConnectOriginateContextError::InvalidProfileId);
    }
    Ok(())
}

fn validate_resource_id(
    value: &str,
    invalid: AmazonConnectOriginateContextError,
    too_large: AmazonConnectOriginateContextError,
) -> Result<(), AmazonConnectOriginateContextError> {
    if value.len() > MAX_CONNECT_RESOURCE_ID_BYTES {
        return Err(too_large);
    }
    if value.is_empty()
        || !value.is_ascii()
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(invalid);
    }
    Ok(())
}

fn validate_display_name(value: &str) -> Result<(), AmazonConnectOriginateContextError> {
    if value.len() > MAX_CONNECT_DISPLAY_NAME_BYTES {
        return Err(AmazonConnectOriginateContextError::DisplayNameTooLarge);
    }
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(AmazonConnectOriginateContextError::InvalidDisplayName);
    }
    Ok(())
}

fn validate_description(value: Option<&str>) -> Result<(), AmazonConnectOriginateContextError> {
    let Some(value) = value else { return Ok(()) };
    if value.len() > MAX_CONNECT_DESCRIPTION_BYTES {
        return Err(AmazonConnectOriginateContextError::DescriptionTooLarge);
    }
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(AmazonConnectOriginateContextError::InvalidDescription);
    }
    Ok(())
}

fn validate_client_token(value: &str) -> Result<(), AmazonConnectOriginateContextError> {
    if value.len() > MAX_CONNECT_CLIENT_TOKEN_BYTES {
        return Err(AmazonConnectOriginateContextError::ClientTokenTooLarge);
    }
    if value.is_empty()
        || !value.is_ascii()
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(AmazonConnectOriginateContextError::InvalidClientToken);
    }
    Ok(())
}

fn validate_attributes(
    attributes: &BTreeMap<String, String>,
) -> Result<(), AmazonConnectOriginateContextError> {
    if attributes.len() > MAX_CONNECT_ATTRIBUTE_COUNT {
        return Err(AmazonConnectOriginateContextError::TooManyAttributes);
    }
    let mut total = 0usize;
    for (key, value) in attributes {
        if key.is_empty()
            || key.len() > MAX_CONNECT_ATTRIBUTE_KEY_BYTES
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(AmazonConnectOriginateContextError::InvalidAttributeKey);
        }
        if value.chars().any(char::is_control) {
            return Err(AmazonConnectOriginateContextError::InvalidAttributeValue);
        }
        total = total
            .checked_add(key.len())
            .and_then(|total| total.checked_add(value.len()))
            .ok_or(AmazonConnectOriginateContextError::AttributesTooLarge)?;
        if total > MAX_ATTRIBUTE_BYTES {
            return Err(AmazonConnectOriginateContextError::AttributesTooLarge);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_context() -> AmazonConnectOriginateContext {
        AmazonConnectOriginateContext::new(
            ConnectProfileId::new("tenant-a").unwrap(),
            AmazonConnectTarget::new("instance-secret", "flow-secret").unwrap(),
            BTreeMap::from([("correlation_id".into(), "customer-secret".into())]),
            "display-secret",
            Some("description-secret".into()),
            ConnectClientToken::new("token-secret").unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn exact_context_reuses_the_stable_request() {
        let context = valid_context();
        let first = context.start_request();
        let second = context.start_request();
        assert_eq!(first.instance_id, second.instance_id);
        assert_eq!(first.contact_flow_id, second.contact_flow_id);
        assert_eq!(first.display_name, second.display_name);
        assert_eq!(first.attributes, second.attributes);
        assert_eq!(first.description, second.description);
        assert_eq!(first.client_token, second.client_token);
        assert_eq!(first.client_token.as_deref(), Some("token-secret"));
    }

    #[test]
    fn context_diagnostics_are_metadata_only() {
        let context = valid_context();
        let diagnostic = format!("{context:?}");
        for secret in [
            "tenant-a",
            "instance-secret",
            "flow-secret",
            "correlation_id",
            "customer-secret",
            "display-secret",
            "description-secret",
            "token-secret",
        ] {
            assert!(!diagnostic.contains(secret), "leaked {secret}");
        }
        assert!(diagnostic.contains("attribute_count: 1"));
    }

    #[test]
    fn all_request_bounds_fail_closed() {
        assert_eq!(
            ConnectProfileId::new("p".repeat(MAX_CONNECT_PROFILE_ID_BYTES + 1)).unwrap_err(),
            AmazonConnectOriginateContextError::ProfileIdTooLarge
        );
        assert_eq!(
            AmazonConnectTarget::new("bad\ninstance", "flow").unwrap_err(),
            AmazonConnectOriginateContextError::InvalidInstanceId
        );
        assert_eq!(
            ConnectClientToken::new("token with space").unwrap_err(),
            AmazonConnectOriginateContextError::InvalidClientToken
        );
        assert!(AmazonConnectOriginateContext::new(
            ConnectProfileId::default(),
            AmazonConnectTarget::new("instance", "flow").unwrap(),
            BTreeMap::new(),
            "display",
            None,
            ConnectClientToken::new("token").unwrap(),
        )
        .is_ok());
        assert_eq!(
            AmazonConnectOriginateContext::new(
                ConnectProfileId::default(),
                AmazonConnectTarget::new("instance", "flow").unwrap(),
                BTreeMap::from([("bad.key".into(), "value".into())]),
                "display",
                None,
                ConnectClientToken::new("token").unwrap(),
            )
            .unwrap_err(),
            AmazonConnectOriginateContextError::InvalidAttributeKey
        );
        assert_eq!(
            AmazonConnectOriginateContext::new(
                ConnectProfileId::default(),
                AmazonConnectTarget::new("instance", "flow").unwrap(),
                BTreeMap::from([("key".into(), "v".repeat(MAX_ATTRIBUTE_BYTES))]),
                "display",
                None,
                ConnectClientToken::new("token").unwrap(),
            )
            .unwrap_err(),
            AmazonConnectOriginateContextError::AttributesTooLarge
        );
    }
}
