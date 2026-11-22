//! SCIM 2.0 provisioning adapter for RVoIP users-core.
//!
//! `rvoip-scim` is an optional enterprise identity lifecycle crate. It maps
//! SCIM Users and Groups onto `rvoip-users-core` users, roles, active state,
//! and external identity links. Authentication is intentionally provider-based:
//! pass an `rvoip-auth-core::BearerValidator` for admin tokens from users-core,
//! Keycloak/OIDC, OAuth2 introspection, or a custom enterprise service.
//!
//! # Configuration
//!
//! [`ScimConfig`] carries the provider id, the read and write scopes, the
//! default roles, and two request limits:
//!
//! - [`ScimConfig::max_request_body_bytes`] (default 2 MiB) caps the size of
//!   any request body the SCIM router reads. [`ScimService::router`] applies
//!   it as its innermost layer, so it holds for SCIM routes whatever body
//!   limit the application sets where it mounts the router: an outer
//!   `DefaultBodyLimit::disable()` or a larger outer limit does not widen it,
//!   and a smaller outer `DefaultBodyLimit` does not narrow it either. Set it
//!   with [`ScimConfig::with_max_request_body_bytes`]. A larger body gets
//!   `413 Payload Too Large`.
//! - [`ScimConfig::max_group_members_per_request`] (default 10,000) caps the
//!   number of `members` one request may carry: POST and PUT of a Group, and
//!   a PATCH that replaces `members`. Set it with
//!   [`ScimConfig::with_max_group_members_per_request`]. Over the cap, the
//!   service returns [`ScimError::InvalidValue`], which the router sends as a
//!   `400` SCIM error with `scimType` `invalidValue` (RFC 7644 §3.12). The
//!   adapter's PATCH supports only `replace` of `members`, which carries the
//!   whole membership, so a larger group needs a larger cap.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use rvoip_auth_core::{BearerAuthError, BearerValidator};
use rvoip_core_traits::identity::IdentityAssurance;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use users_core::{
    AuthenticationService, CreateUserRequest, Error as UsersError, ExternalIdentity,
    TokenIssueContext, UpdateUserRequest, UpsertExternalIdentityRequest, User, UserFilter,
};
use uuid::Uuid;

const SCIM_USER_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:User";
const SCIM_GROUP_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:Group";
const SCIM_ERROR_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:Error";

/// Default for [`ScimConfig::max_request_body_bytes`]: 2 MiB, the same as
/// axum's own default body limit.
pub const DEFAULT_MAX_REQUEST_BODY_BYTES: usize = 2 * 1024 * 1024;

/// Default for [`ScimConfig::max_group_members_per_request`].
pub const DEFAULT_MAX_GROUP_MEMBERS_PER_REQUEST: usize = 10_000;

/// SCIM adapter error.
#[derive(Debug, Error)]
pub enum ScimError {
    #[error("SCIM configuration error: {0}")]
    Config(String),
    #[error("SCIM authorization failed: {0}")]
    Unauthorized(String),
    #[error("SCIM resource was not found")]
    NotFound,
    #[error("SCIM conflict: {0}")]
    Conflict(String),
    /// A request value is outside what the service accepts, such as a Group
    /// carrying more members than
    /// [`ScimConfig::max_group_members_per_request`]. The router sends it as
    /// a `400` SCIM error with `scimType` `invalidValue`.
    #[error("SCIM invalid value: {0}")]
    InvalidValue(String),
    #[error("users-core error: {0}")]
    Users(#[from] UsersError),
}

pub type Result<T> = std::result::Result<T, ScimError>;

/// SCIM service configuration.
#[derive(Debug, Clone)]
pub struct ScimConfig {
    /// Stable provider id stored in users-core external identity links.
    pub provider_id: String,
    /// Scope required for read operations.
    pub read_scope: String,
    /// Scope required for write operations.
    pub write_scope: String,
    /// Roles assigned to provisioned users when SCIM groups do not map to
    /// users-core's built-in role set.
    pub default_roles: Vec<String>,
    /// Largest request body, in bytes, that the SCIM router reads. Default
    /// [`DEFAULT_MAX_REQUEST_BODY_BYTES`] (2 MiB).
    ///
    /// [`ScimService::router`] applies it as its innermost layer, so an outer
    /// `DefaultBodyLimit` where the router is mounted (disabled, larger, or
    /// smaller) does not change it for SCIM routes. A larger body gets
    /// `413 Payload Too Large`.
    pub max_request_body_bytes: usize,
    /// Most Group `members` one request may carry. Default
    /// [`DEFAULT_MAX_GROUP_MEMBERS_PER_REQUEST`] (10,000).
    ///
    /// Applies to creating and replacing a Group and to a PATCH that replaces
    /// `members`, through the router and the direct service methods alike.
    /// Over the cap, the request fails with [`ScimError::InvalidValue`].
    pub max_group_members_per_request: usize,
}

impl ScimConfig {
    pub fn new(provider_id: impl Into<String>) -> Self {
        Self {
            provider_id: provider_id.into(),
            read_scope: "scim.read".to_string(),
            write_scope: "scim.write".to_string(),
            default_roles: vec!["user".to_string()],
            max_request_body_bytes: DEFAULT_MAX_REQUEST_BODY_BYTES,
            max_group_members_per_request: DEFAULT_MAX_GROUP_MEMBERS_PER_REQUEST,
        }
    }

    pub fn with_read_scope(mut self, scope: impl Into<String>) -> Self {
        self.read_scope = scope.into();
        self
    }

    pub fn with_write_scope(mut self, scope: impl Into<String>) -> Self {
        self.write_scope = scope.into();
        self
    }

    pub fn with_default_roles(
        mut self,
        roles: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.default_roles = roles.into_iter().map(Into::into).collect();
        self
    }

    /// Set [`ScimConfig::max_request_body_bytes`].
    pub fn with_max_request_body_bytes(mut self, bytes: usize) -> Self {
        self.max_request_body_bytes = bytes;
        self
    }

    /// Set [`ScimConfig::max_group_members_per_request`].
    pub fn with_max_group_members_per_request(mut self, members: usize) -> Self {
        self.max_group_members_per_request = members;
        self
    }
}

impl Default for ScimConfig {
    fn default() -> Self {
        Self::new("scim")
    }
}

/// Axum-compatible SCIM service.
#[derive(Clone)]
pub struct ScimService {
    auth_service: Arc<AuthenticationService>,
    admin_bearer: Arc<dyn BearerValidator>,
    config: ScimConfig,
    groups: Arc<RwLock<BTreeMap<String, ScimGroup>>>,
}

impl ScimService {
    pub fn new(
        auth_service: Arc<AuthenticationService>,
        admin_bearer: Arc<dyn BearerValidator>,
        config: ScimConfig,
    ) -> Result<Self> {
        if auth_service.enterprise_identity_store().is_none() {
            return Err(ScimError::Config(
                "AuthenticationService must be configured with an EnterpriseIdentityStore"
                    .to_string(),
            ));
        }
        if config.provider_id.trim().is_empty() {
            return Err(ScimError::Config("provider_id is required".to_string()));
        }
        Ok(Self {
            auth_service,
            admin_bearer,
            config,
            groups: Arc::new(RwLock::new(BTreeMap::new())),
        })
    }

    /// Return an Axum router for SCIM service metadata, Users, and Groups.
    ///
    /// The router enforces [`ScimConfig::max_request_body_bytes`] as its
    /// innermost layer, so the limit holds whatever `DefaultBodyLimit` the
    /// application applies where it mounts the router.
    pub fn router(self: Arc<Self>) -> Router {
        let max_request_body_bytes = self.config.max_request_body_bytes;
        Router::new()
            .route(
                "/ServiceProviderConfig",
                get(|| async { Json(ScimServiceProviderConfig::default()) }),
            )
            .route(
                "/Schemas",
                get(|| async { Json(ScimListResponse::from_values(scim_schemas())) }),
            )
            .route("/Users", get(list_users_handler).post(create_user_handler))
            .route(
                "/Users/{id}",
                get(get_user_handler)
                    .put(replace_user_handler)
                    .patch(patch_user_handler)
                    .delete(delete_user_handler),
            )
            .route(
                "/Groups",
                get(list_groups_handler).post(create_group_handler),
            )
            .route(
                "/Groups/{id}",
                get(get_group_handler)
                    .put(replace_group_handler)
                    .patch(patch_group_handler)
                    .delete(delete_group_handler),
            )
            // Innermost: DefaultBodyLimit sets a request extension, and the
            // layer nearest the handler writes it last, so an outer
            // `DefaultBodyLimit` cannot change the SCIM limit.
            .layer(DefaultBodyLimit::max(max_request_body_bytes))
            .with_state(self)
    }

    /// Validate a SCIM admin Bearer token for a required scope.
    pub async fn authorize(&self, authorization: &str, required_scope: &str) -> Result<()> {
        let token = authorization
            .strip_prefix("Bearer ")
            .or_else(|| authorization.strip_prefix("bearer "))
            .ok_or_else(|| ScimError::Unauthorized("Bearer token is required".to_string()))?
            .trim();
        let assurance = self
            .admin_bearer
            .validate(token)
            .await
            .map_err(map_bearer_error)?;
        let scopes = scopes_from_assurance(&assurance);
        if scopes.iter().any(|scope| {
            scope == required_scope
                || scope == "scim.*"
                || (required_scope == self.config.read_scope && scope == &self.config.write_scope)
        }) {
            Ok(())
        } else {
            Err(ScimError::Unauthorized(format!(
                "required scope `{required_scope}` was not present"
            )))
        }
    }

    pub async fn create_user_authorized(
        &self,
        authorization: &str,
        user: ScimUser,
    ) -> Result<ScimUser> {
        self.authorize(authorization, &self.config.write_scope)
            .await?;
        self.create_or_update_user(user).await
    }

    pub async fn get_user_authorized(&self, authorization: &str, id: &str) -> Result<ScimUser> {
        self.authorize(authorization, &self.config.read_scope)
            .await?;
        self.get_user(id).await
    }

    pub async fn list_users_authorized(
        &self,
        authorization: &str,
        request: ScimListRequest,
    ) -> Result<ScimListResponse<ScimUser>> {
        self.authorize(authorization, &self.config.read_scope)
            .await?;
        self.list_users(request).await
    }

    pub async fn patch_user_authorized(
        &self,
        authorization: &str,
        id: &str,
        patch: ScimPatchRequest,
    ) -> Result<ScimUser> {
        self.authorize(authorization, &self.config.write_scope)
            .await?;
        self.patch_user(id, patch).await
    }

    pub async fn delete_user_authorized(&self, authorization: &str, id: &str) -> Result<()> {
        self.authorize(authorization, &self.config.write_scope)
            .await?;
        self.deactivate_user(id).await
    }

    pub async fn create_group_authorized(
        &self,
        authorization: &str,
        group: ScimGroup,
    ) -> Result<ScimGroup> {
        self.authorize(authorization, &self.config.write_scope)
            .await?;
        self.create_or_update_group(group).await
    }

    pub async fn get_group_authorized(&self, authorization: &str, id: &str) -> Result<ScimGroup> {
        self.authorize(authorization, &self.config.read_scope)
            .await?;
        self.get_group(id).await
    }

    pub async fn list_groups_authorized(
        &self,
        authorization: &str,
        request: ScimListRequest,
    ) -> Result<ScimListResponse<ScimGroup>> {
        self.authorize(authorization, &self.config.read_scope)
            .await?;
        self.list_groups(request).await
    }

    pub async fn patch_group_authorized(
        &self,
        authorization: &str,
        id: &str,
        patch: ScimPatchRequest,
    ) -> Result<ScimGroup> {
        self.authorize(authorization, &self.config.write_scope)
            .await?;
        self.patch_group(id, patch).await
    }

    pub async fn delete_group_authorized(&self, authorization: &str, id: &str) -> Result<()> {
        self.authorize(authorization, &self.config.write_scope)
            .await?;
        self.delete_group(id).await
    }

    /// Create or replace a SCIM user without performing Bearer auth.
    ///
    /// This is intended for trusted internal callers or HTTP handlers that
    /// already authenticated the request.
    pub async fn create_or_update_user(&self, scim_user: ScimUser) -> Result<ScimUser> {
        let external_subject = scim_user
            .external_id
            .clone()
            .or_else(|| scim_user.id.clone())
            .unwrap_or_else(|| scim_user.user_name.clone());
        let username = scim_user.user_name.trim();
        if username.is_empty() {
            return Err(ScimError::Config("SCIM userName is required".to_string()));
        }

        let enterprise_store = self.enterprise_store()?;
        let existing_link = enterprise_store
            .get_external_identity(&self.config.provider_id, &external_subject)
            .await?;
        let user = if let Some(link) = existing_link {
            self.update_users_core_user(&link.user_id, &scim_user)
                .await?
        } else if let Some(existing) = self
            .auth_service
            .user_store()
            .get_user_by_username(username)
            .await?
        {
            self.update_users_core_user(&existing.id, &scim_user)
                .await?
        } else {
            self.create_users_core_user(&scim_user).await?
        };

        let groups = scim_user
            .groups
            .iter()
            .map(|group| group.display.clone().unwrap_or_else(|| group.value.clone()))
            .collect::<Vec<_>>();
        enterprise_store
            .upsert_external_identity(UpsertExternalIdentityRequest {
                provider_id: self.config.provider_id.clone(),
                external_subject,
                user_id: user.id.clone(),
                email: primary_email(&scim_user),
                username: Some(scim_user.user_name.clone()),
                display_name: scim_user.display_name.clone(),
                groups,
                active: scim_user.active.unwrap_or(true),
            })
            .await?;

        self.user_to_scim(&user).await
    }

    pub async fn get_user(&self, id: &str) -> Result<ScimUser> {
        let user = self
            .auth_service
            .user_store()
            .get_user(id)
            .await?
            .ok_or(ScimError::NotFound)?;
        self.user_to_scim(&user).await
    }

    pub async fn list_users(&self, request: ScimListRequest) -> Result<ScimListResponse<ScimUser>> {
        let users = self
            .auth_service
            .user_store()
            .list_users(UserFilter {
                active: request.active,
                search: request.filter_search(),
                limit: request.count,
                offset: request.start_index.map(|value| value.saturating_sub(1)),
                role: None,
            })
            .await?;
        let total_results = users.len() as u32;
        let mut resources = Vec::with_capacity(users.len());
        for user in users {
            resources.push(self.user_to_scim(&user).await?);
        }
        Ok(ScimListResponse {
            schemas: vec!["urn:ietf:params:scim:api:messages:2.0:ListResponse".to_string()],
            total_results,
            start_index: request.start_index.unwrap_or(1),
            items_per_page: resources.len() as u32,
            resources,
        })
    }

    pub async fn patch_user(&self, id: &str, patch: ScimPatchRequest) -> Result<ScimUser> {
        let mut active = None;
        for operation in patch.operations {
            if operation.op.eq_ignore_ascii_case("replace")
                && operation
                    .path
                    .as_deref()
                    .is_some_and(|path| path.eq_ignore_ascii_case("active"))
            {
                active = operation.value.as_bool();
            }
        }
        if active.is_none() {
            return Err(ScimError::Config(
                "only PATCH replace active is supported by the adapter helper".to_string(),
            ));
        }
        let user = self
            .auth_service
            .user_store()
            .update_user(
                id,
                UpdateUserRequest {
                    email: None,
                    display_name: None,
                    roles: None,
                    active,
                },
            )
            .await?;
        self.user_to_scim(&user).await
    }

    pub async fn deactivate_user(&self, id: &str) -> Result<()> {
        self.auth_service
            .user_store()
            .update_user(
                id,
                UpdateUserRequest {
                    email: None,
                    display_name: None,
                    roles: None,
                    active: Some(false),
                },
            )
            .await?;
        Ok(())
    }

    /// Create or replace a SCIM group without performing Bearer auth.
    ///
    /// users-core currently models authorization groups as roles; this adapter
    /// keeps SCIM group resources in memory and maps group display names to
    /// users-core roles when users are provisioned.
    ///
    /// Fails with [`ScimError::InvalidValue`] when the group carries more than
    /// [`ScimConfig::max_group_members_per_request`] members.
    pub async fn create_or_update_group(&self, mut group: ScimGroup) -> Result<ScimGroup> {
        if group.display_name.trim().is_empty() {
            return Err(ScimError::Config(
                "SCIM group displayName is required".to_string(),
            ));
        }
        self.check_group_member_count(group.members.len())?;
        let id = group
            .id
            .clone()
            .or_else(|| group.external_id.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        group.id = Some(id.clone());
        if group.schemas.is_empty() {
            group.schemas.push(SCIM_GROUP_SCHEMA.to_string());
        }
        self.groups.write().insert(id, group.clone());
        Ok(group)
    }

    pub async fn get_group(&self, id: &str) -> Result<ScimGroup> {
        self.groups
            .read()
            .get(id)
            .cloned()
            .ok_or(ScimError::NotFound)
    }

    pub async fn list_groups(
        &self,
        request: ScimListRequest,
    ) -> Result<ScimListResponse<ScimGroup>> {
        let mut groups = self.groups.read().values().cloned().collect::<Vec<_>>();
        if let Some(search) = request.filter_search() {
            groups.retain(|group| {
                group.id.as_deref() == Some(search.as_str())
                    || group.external_id.as_deref() == Some(search.as_str())
                    || group.display_name == search
            });
        }
        let total_results = groups.len() as u32;
        let offset = request.start_index.unwrap_or(1).saturating_sub(1) as usize;
        let limit = request.count.unwrap_or(total_results) as usize;
        let resources = groups
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect::<Vec<_>>();
        Ok(ScimListResponse {
            schemas: vec!["urn:ietf:params:scim:api:messages:2.0:ListResponse".to_string()],
            total_results,
            start_index: request.start_index.unwrap_or(1),
            items_per_page: resources.len() as u32,
            resources,
        })
    }

    /// Apply a SCIM PATCH to a group without performing Bearer auth.
    ///
    /// Fails with [`ScimError::InvalidValue`] when a `replace` of `members`
    /// carries more than [`ScimConfig::max_group_members_per_request`]
    /// members.
    pub async fn patch_group(&self, id: &str, patch: ScimPatchRequest) -> Result<ScimGroup> {
        let mut group = self.get_group(id).await?;
        for operation in patch.operations {
            if !operation.op.eq_ignore_ascii_case("replace") {
                continue;
            }
            match operation
                .path
                .as_deref()
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("displayname") => {
                    if let Some(value) = operation.value.as_str() {
                        group.display_name = value.to_string();
                    }
                }
                Some("members") => {
                    let members: Vec<ScimGroupRef> = serde_json::from_value(operation.value)
                        .map_err(|err| {
                            ScimError::Config(format!("invalid SCIM group members: {err}"))
                        })?;
                    self.check_group_member_count(members.len())?;
                    group.members = members;
                }
                _ => {}
            }
        }
        self.create_or_update_group(group).await
    }

    pub async fn delete_group(&self, id: &str) -> Result<()> {
        self.groups
            .write()
            .remove(id)
            .map(|_| ())
            .ok_or(ScimError::NotFound)
    }

    /// Issue users-core tokens for a provisioned user after an external login
    /// flow has authenticated the same linked external subject.
    pub async fn issue_tokens_for_external_subject(
        &self,
        external_subject: &str,
    ) -> Result<users_core::AuthenticationResult> {
        let link = self
            .enterprise_store()?
            .get_external_identity(&self.config.provider_id, external_subject)
            .await?
            .ok_or(ScimError::NotFound)?;
        Ok(self
            .auth_service
            .issue_tokens_for_user(
                &link.user_id,
                TokenIssueContext::external_identity(
                    "scim",
                    &self.config.provider_id,
                    external_subject,
                ),
            )
            .await?)
    }

    fn check_group_member_count(&self, members: usize) -> Result<()> {
        let max = self.config.max_group_members_per_request;
        if members > max {
            return Err(ScimError::InvalidValue(format!(
                "a request may carry at most {max} group members; this one carries {members}"
            )));
        }
        Ok(())
    }

    fn enterprise_store(&self) -> Result<&Arc<dyn users_core::EnterpriseIdentityStore>> {
        self.auth_service
            .enterprise_identity_store()
            .ok_or_else(|| {
                ScimError::Config("EnterpriseIdentityStore is not configured".to_string())
            })
    }

    async fn create_users_core_user(&self, scim_user: &ScimUser) -> Result<User> {
        self.auth_service
            .create_user(CreateUserRequest {
                username: scim_user.user_name.clone(),
                password: provisioning_password(),
                email: primary_email(scim_user),
                display_name: scim_user.display_name.clone(),
                roles: self.roles_from_groups(scim_user),
            })
            .await
            .map_err(Into::into)
    }

    async fn update_users_core_user(&self, user_id: &str, scim_user: &ScimUser) -> Result<User> {
        self.auth_service
            .user_store()
            .update_user(
                user_id,
                UpdateUserRequest {
                    email: primary_email(scim_user),
                    display_name: scim_user.display_name.clone(),
                    roles: Some(self.roles_from_groups(scim_user)),
                    active: scim_user.active,
                },
            )
            .await
            .map_err(Into::into)
    }

    async fn user_to_scim(&self, user: &User) -> Result<ScimUser> {
        let link = self.primary_link_for_user(&user.id).await?;
        Ok(ScimUser {
            schemas: vec![SCIM_USER_SCHEMA.to_string()],
            id: Some(user.id.clone()),
            external_id: link.as_ref().map(|link| link.external_subject.clone()),
            user_name: user.username.clone(),
            display_name: user.display_name.clone(),
            active: Some(user.active),
            emails: user.email.as_ref().map(|email| {
                vec![ScimEmail {
                    value: email.clone(),
                    primary: Some(true),
                    type_: Some("work".to_string()),
                }]
            }),
            groups: link
                .map(|link| {
                    link.groups
                        .into_iter()
                        .map(|group| ScimGroupRef {
                            value: group.clone(),
                            display: Some(group),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            meta: Some(ScimMeta {
                resource_type: "User".to_string(),
                created: Some(user.created_at),
                last_modified: Some(user.updated_at),
            }),
        })
    }

    async fn primary_link_for_user(&self, user_id: &str) -> Result<Option<ExternalIdentity>> {
        let links = self
            .enterprise_store()?
            .list_external_identities_for_user(user_id)
            .await?;
        Ok(links
            .into_iter()
            .find(|link| link.provider_id == self.config.provider_id))
    }

    fn roles_from_groups(&self, scim_user: &ScimUser) -> Vec<String> {
        let allowed = ["user", "admin", "moderator", "guest"];
        let mut roles = BTreeSet::new();
        for role in &self.config.default_roles {
            if allowed.contains(&role.as_str()) {
                roles.insert(role.clone());
            }
        }
        for group in &scim_user.groups {
            let candidate = group.display.as_deref().unwrap_or(&group.value);
            if allowed.contains(&candidate) {
                roles.insert(candidate.to_string());
            }
        }
        if roles.is_empty() {
            roles.insert("user".to_string());
        }
        roles.into_iter().collect()
    }
}

fn provisioning_password() -> String {
    // Separate every alphanumeric character so a random UUID cannot contain a
    // repeated/sequential four-character run or a valid SCIM username. The
    // fixed prefix supplies every required character class and, together with
    // the UUID version/variant nibbles, at least six unique characters.
    let token = Uuid::new_v4().simple().to_string();
    let mut password = String::with_capacity(6 + token.len() * 2);
    password.push_str("Q!a!7!");
    for character in token.chars() {
        password.push(character);
        password.push('!');
    }
    password
}

// Handler errors are full responses: most are the `(StatusCode, String)`
// plain-text bodies the router has always sent, and `InvalidValue` is a SCIM
// JSON error (RFC 7644 §3.12).
type JsonHandlerResult<T> = std::result::Result<Json<T>, Response>;
type EmptyHandlerResult = std::result::Result<StatusCode, Response>;

async fn create_user_handler(
    State(service): State<Arc<ScimService>>,
    headers: HeaderMap,
    Json(user): Json<ScimUser>,
) -> JsonHandlerResult<ScimUser> {
    let auth = authorization_header(&headers)?;
    service
        .create_user_authorized(auth, user)
        .await
        .map(Json)
        .map_err(scim_http_error)
}

async fn list_users_handler(
    State(service): State<Arc<ScimService>>,
    headers: HeaderMap,
    Query(request): Query<ScimListRequest>,
) -> JsonHandlerResult<ScimListResponse<ScimUser>> {
    let auth = authorization_header(&headers)?;
    service
        .list_users_authorized(auth, request)
        .await
        .map(Json)
        .map_err(scim_http_error)
}

async fn get_user_handler(
    State(service): State<Arc<ScimService>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> JsonHandlerResult<ScimUser> {
    let auth = authorization_header(&headers)?;
    service
        .get_user_authorized(auth, &id)
        .await
        .map(Json)
        .map_err(scim_http_error)
}

async fn replace_user_handler(
    State(service): State<Arc<ScimService>>,
    headers: HeaderMap,
    Path(_id): Path<String>,
    Json(user): Json<ScimUser>,
) -> JsonHandlerResult<ScimUser> {
    let auth = authorization_header(&headers)?;
    service
        .create_user_authorized(auth, user)
        .await
        .map(Json)
        .map_err(scim_http_error)
}

async fn patch_user_handler(
    State(service): State<Arc<ScimService>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(patch): Json<ScimPatchRequest>,
) -> JsonHandlerResult<ScimUser> {
    let auth = authorization_header(&headers)?;
    service
        .patch_user_authorized(auth, &id, patch)
        .await
        .map(Json)
        .map_err(scim_http_error)
}

async fn delete_user_handler(
    State(service): State<Arc<ScimService>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> EmptyHandlerResult {
    let auth = authorization_header(&headers)?;
    service
        .delete_user_authorized(auth, &id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(scim_http_error)
}

async fn create_group_handler(
    State(service): State<Arc<ScimService>>,
    headers: HeaderMap,
    Json(group): Json<ScimGroup>,
) -> JsonHandlerResult<ScimGroup> {
    let auth = authorization_header(&headers)?;
    service
        .create_group_authorized(auth, group)
        .await
        .map(Json)
        .map_err(scim_http_error)
}

async fn list_groups_handler(
    State(service): State<Arc<ScimService>>,
    headers: HeaderMap,
    Query(request): Query<ScimListRequest>,
) -> JsonHandlerResult<ScimListResponse<ScimGroup>> {
    let auth = authorization_header(&headers)?;
    service
        .list_groups_authorized(auth, request)
        .await
        .map(Json)
        .map_err(scim_http_error)
}

async fn get_group_handler(
    State(service): State<Arc<ScimService>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> JsonHandlerResult<ScimGroup> {
    let auth = authorization_header(&headers)?;
    service
        .get_group_authorized(auth, &id)
        .await
        .map(Json)
        .map_err(scim_http_error)
}

async fn replace_group_handler(
    State(service): State<Arc<ScimService>>,
    headers: HeaderMap,
    Path(_id): Path<String>,
    Json(group): Json<ScimGroup>,
) -> JsonHandlerResult<ScimGroup> {
    let auth = authorization_header(&headers)?;
    service
        .create_group_authorized(auth, group)
        .await
        .map(Json)
        .map_err(scim_http_error)
}

async fn patch_group_handler(
    State(service): State<Arc<ScimService>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(patch): Json<ScimPatchRequest>,
) -> JsonHandlerResult<ScimGroup> {
    let auth = authorization_header(&headers)?;
    service
        .patch_group_authorized(auth, &id, patch)
        .await
        .map(Json)
        .map_err(scim_http_error)
}

async fn delete_group_handler(
    State(service): State<Arc<ScimService>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> EmptyHandlerResult {
    let auth = authorization_header(&headers)?;
    service
        .delete_group_authorized(auth, &id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(scim_http_error)
}

fn authorization_header(headers: &HeaderMap) -> std::result::Result<&str, Response> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                "Authorization Bearer token is required".to_string(),
            )
                .into_response()
        })
}

fn scim_http_error(err: ScimError) -> Response {
    let status = match err {
        ScimError::InvalidValue(detail) => {
            return scim_error_response(StatusCode::BAD_REQUEST, "invalidValue", detail);
        }
        ScimError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
        ScimError::NotFound => StatusCode::NOT_FOUND,
        ScimError::Conflict(_) => StatusCode::CONFLICT,
        ScimError::Config(_) | ScimError::Users(_) => StatusCode::BAD_REQUEST,
    };
    (status, err.to_string()).into_response()
}

/// A SCIM error response body (RFC 7644 §3.12), sent as JSON like the rest of
/// the router's responses.
fn scim_error_response(status: StatusCode, scim_type: &str, detail: String) -> Response {
    (
        status,
        Json(serde_json::json!({
            "schemas": [SCIM_ERROR_SCHEMA],
            "scimType": scim_type,
            "detail": detail,
            "status": status.as_u16().to_string(),
        })),
    )
        .into_response()
}

/// SCIM User resource subset used by the adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimUser {
    #[serde(default)]
    pub schemas: Vec<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "externalId", default)]
    pub external_id: Option<String>,
    #[serde(rename = "userName")]
    pub user_name: String,
    #[serde(rename = "displayName", default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub active: Option<bool>,
    #[serde(default)]
    pub emails: Option<Vec<ScimEmail>>,
    #[serde(default)]
    pub groups: Vec<ScimGroupRef>,
    #[serde(default)]
    pub meta: Option<ScimMeta>,
}

/// SCIM Group resource subset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimGroup {
    #[serde(default)]
    pub schemas: Vec<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "externalId", default)]
    pub external_id: Option<String>,
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(default)]
    pub members: Vec<ScimGroupRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimGroupRef {
    pub value: String,
    #[serde(default)]
    pub display: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimEmail {
    pub value: String,
    #[serde(default)]
    pub primary: Option<bool>,
    #[serde(rename = "type", default)]
    pub type_: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimMeta {
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    #[serde(default)]
    pub created: Option<DateTime<Utc>>,
    #[serde(rename = "lastModified", default)]
    pub last_modified: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ScimListRequest {
    #[serde(rename = "startIndex")]
    pub start_index: Option<u32>,
    pub count: Option<u32>,
    pub filter: Option<String>,
    pub active: Option<bool>,
}

impl ScimListRequest {
    fn filter_search(&self) -> Option<String> {
        let filter = self.filter.as_ref()?;
        for prefix in ["userName eq ", "externalId eq ", "id eq "] {
            if let Some(value) = filter.strip_prefix(prefix) {
                return Some(value.trim_matches('"').to_string());
            }
        }
        None
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ScimListResponse<T> {
    pub schemas: Vec<String>,
    #[serde(rename = "totalResults")]
    pub total_results: u32,
    #[serde(rename = "startIndex")]
    pub start_index: u32,
    #[serde(rename = "itemsPerPage")]
    pub items_per_page: u32,
    #[serde(rename = "Resources")]
    pub resources: Vec<T>,
}

impl<T> ScimListResponse<T> {
    fn from_values(resources: Vec<T>) -> Self {
        Self {
            schemas: vec!["urn:ietf:params:scim:api:messages:2.0:ListResponse".to_string()],
            total_results: resources.len() as u32,
            start_index: 1,
            items_per_page: resources.len() as u32,
            resources,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScimPatchRequest {
    #[serde(rename = "Operations")]
    pub operations: Vec<ScimPatchOperation>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScimPatchOperation {
    pub op: String,
    #[serde(default)]
    pub path: Option<String>,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScimServiceProviderConfig {
    pub schemas: Vec<String>,
    #[serde(rename = "patch")]
    pub patch_config: ScimSupported,
    #[serde(rename = "bulk")]
    pub bulk_config: ScimSupported,
    #[serde(rename = "filter")]
    pub filter_config: ScimFilterConfig,
    #[serde(rename = "changePassword")]
    pub change_password_config: ScimSupported,
    pub sort: ScimSupported,
    pub etag: ScimSupported,
    pub authentication_schemes: Vec<ScimAuthScheme>,
}

impl Default for ScimServiceProviderConfig {
    fn default() -> Self {
        Self {
            schemas: vec![
                "urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig".to_string(),
            ],
            patch_config: ScimSupported { supported: true },
            bulk_config: ScimSupported { supported: false },
            filter_config: ScimFilterConfig {
                supported: true,
                max_results: 200,
            },
            change_password_config: ScimSupported { supported: false },
            sort: ScimSupported { supported: false },
            etag: ScimSupported { supported: false },
            authentication_schemes: vec![ScimAuthScheme {
                type_: "oauthbearertoken".to_string(),
                name: "OAuth Bearer Token".to_string(),
                description: "OAuth2 Bearer token accepted by the configured BearerValidator"
                    .to_string(),
                primary: true,
            }],
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ScimSupported {
    pub supported: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScimFilterConfig {
    pub supported: bool,
    #[serde(rename = "maxResults")]
    pub max_results: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScimAuthScheme {
    #[serde(rename = "type")]
    pub type_: String,
    pub name: String,
    pub description: String,
    pub primary: bool,
}

fn primary_email(user: &ScimUser) -> Option<String> {
    let emails = user.emails.as_ref()?;
    emails
        .iter()
        .find(|email| email.primary.unwrap_or(false))
        .or_else(|| emails.first())
        .map(|email| email.value.clone())
}

fn scopes_from_assurance(assurance: &IdentityAssurance) -> Vec<String> {
    match assurance {
        IdentityAssurance::TaskScoped { scopes, .. }
        | IdentityAssurance::UserAuthorized { scopes, .. } => scopes.clone(),
        _ => Vec::new(),
    }
}

fn map_bearer_error(err: BearerAuthError) -> ScimError {
    match err {
        BearerAuthError::Empty => ScimError::Unauthorized("empty Bearer token".to_string()),
        BearerAuthError::Invalid(reason) => {
            ScimError::Unauthorized(format!("invalid Bearer token: {reason}"))
        }
        BearerAuthError::Unavailable(reason) => {
            ScimError::Unauthorized(format!("Bearer validator unavailable: {reason}"))
        }
    }
}

fn scim_schemas() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "id": SCIM_USER_SCHEMA,
            "name": "User",
            "description": "RVoIP users-core user"
        }),
        serde_json::json!({
            "id": SCIM_GROUP_SCHEMA,
            "name": "Group",
            "description": "RVoIP users-core group/role mapping"
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use rvoip_core_traits::ids::IdentityId;
    use users_core::config::{PasswordConfig, TlsSettings};
    use users_core::jwt::JwtConfig;
    use users_core::validation::PasswordValidator;
    use users_core::{init, UsersConfig};

    #[derive(Clone)]
    struct StaticBearer {
        scopes: Vec<String>,
    }

    #[async_trait]
    impl BearerValidator for StaticBearer {
        async fn validate(
            &self,
            token: &str,
        ) -> std::result::Result<IdentityAssurance, BearerAuthError> {
            if token != "admin-token" {
                return Err(BearerAuthError::Invalid("bad token".to_string()));
            }
            let identity = IdentityId::from_string("scim-admin");
            Ok(IdentityAssurance::UserAuthorized {
                identity: identity.clone(),
                user_id: identity,
                scopes: self.scopes.clone(),
            })
        }
    }

    fn test_config(db_url: String) -> UsersConfig {
        UsersConfig {
            database_url: db_url,
            jwt: JwtConfig {
                issuer: "https://users.rvoip.local".to_string(),
                audience: vec!["rvoip-app".to_string()],
                access_ttl_seconds: 300,
                refresh_ttl_seconds: 3600,
                algorithm: "HS256".to_string(),
                tenant_id: None,
                signing_key: Some("scim-test-secret".to_string()),
            },
            password: PasswordConfig {
                min_length: 12,
                require_uppercase: true,
                require_lowercase: true,
                require_numbers: true,
                require_special: false,
                argon2_memory_cost: 1024,
                argon2_time_cost: 2,
                argon2_parallelism: 1,
            },
            api_bind_address: "127.0.0.1:0".to_string(),
            tls: TlsSettings::default(),
        }
    }

    async fn test_service(scopes: Vec<&str>) -> (tempfile::TempDir, ScimService) {
        test_service_with_config(scopes, ScimConfig::new("okta")).await
    }

    async fn test_service_with_config(
        scopes: Vec<&str>,
        config: ScimConfig,
    ) -> (tempfile::TempDir, ScimService) {
        let temp = tempfile::tempdir().unwrap();
        let db_url = format!(
            "sqlite://{}?mode=rwc",
            temp.path().join("users.db").display()
        );
        let auth = Arc::new(init(test_config(db_url)).await.unwrap());
        let bearer = Arc::new(StaticBearer {
            scopes: scopes.into_iter().map(str::to_string).collect(),
        });
        let service = ScimService::new(auth, bearer, config).unwrap();
        (temp, service)
    }

    fn members(count: usize) -> Vec<ScimGroupRef> {
        (0..count)
            .map(|index| ScimGroupRef {
                value: format!("user-{index}"),
                display: None,
            })
            .collect()
    }

    fn group_json(id: &str, display_name: &str, member_count: usize) -> serde_json::Value {
        serde_json::json!({
            "schemas": [SCIM_GROUP_SCHEMA],
            "id": id,
            "displayName": display_name,
            "members": members(member_count),
        })
    }

    fn members_patch_json(member_count: usize) -> serde_json::Value {
        serde_json::json!({
            "schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
            "Operations": [{
                "op": "replace",
                "path": "members",
                "value": members(member_count),
            }],
        })
    }

    async fn send(
        app: &Router,
        method: &str,
        uri: &str,
        body: Vec<u8>,
    ) -> (StatusCode, Option<String>, Vec<u8>) {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("authorization", "Bearer admin-token")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .map(|value| value.to_str().unwrap().to_string());
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec();
        (status, content_type, body)
    }

    fn assert_invalid_value(
        what: &str,
        (status, content_type, body): (StatusCode, Option<String>, Vec<u8>),
    ) {
        assert_eq!(status, StatusCode::BAD_REQUEST, "{what}");
        assert_eq!(content_type.as_deref(), Some("application/json"), "{what}");
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            body["schemas"],
            serde_json::json!([SCIM_ERROR_SCHEMA]),
            "{what}"
        );
        assert_eq!(body["scimType"], "invalidValue", "{what}");
        assert_eq!(body["status"], "400", "{what}");
        assert_eq!(
            body["detail"], "a request may carry at most 3 group members; this one carries 4",
            "{what}"
        );
    }

    #[test]
    fn config_defaults_limit_bodies_to_two_mib_and_groups_to_ten_thousand_members() {
        let config = ScimConfig::new("okta");
        assert_eq!(config.max_request_body_bytes, 2 * 1024 * 1024);
        assert_eq!(config.max_group_members_per_request, 10_000);
        assert_eq!(
            ScimConfig::default().max_request_body_bytes,
            2 * 1024 * 1024
        );
        assert_eq!(ScimConfig::default().max_group_members_per_request, 10_000);

        let config = ScimConfig::new("okta")
            .with_max_request_body_bytes(4096)
            .with_max_group_members_per_request(3);
        assert_eq!(config.max_request_body_bytes, 4096);
        assert_eq!(config.max_group_members_per_request, 3);
    }

    /// The SCIM body limit is the router's innermost layer, so an application
    /// that disables or raises axum's body limit where it mounts the router
    /// cannot widen it.
    #[tokio::test]
    async fn body_limit_holds_under_an_outer_router_that_disables_or_raises_it() {
        const LIMIT: usize = 4 * 1024;
        let (_temp, service) = test_service_with_config(
            vec!["scim.write"],
            ScimConfig::new("okta").with_max_request_body_bytes(LIMIT),
        )
        .await;
        let scim = Arc::new(service).router();
        let oversized_name = "x".repeat(2 * LIMIT);
        let oversized_group = serde_json::to_vec(&group_json("g-1", &oversized_name, 0)).unwrap();
        let oversized_patch = serde_json::to_vec(&serde_json::json!({
            "Operations": [{"op": "replace", "path": "displayName", "value": oversized_name}],
        }))
        .unwrap();
        assert!(oversized_group.len() > LIMIT && oversized_patch.len() > LIMIT);
        let group = serde_json::to_vec(&group_json("g-1", "supervisors", 1)).unwrap();
        let patch = serde_json::to_vec(&serde_json::json!({
            "Operations": [{"op": "replace", "path": "displayName", "value": "agents"}],
        }))
        .unwrap();

        for (outer_name, outer_limit) in [
            ("disabled", DefaultBodyLimit::disable()),
            ("raised to 64 MiB", DefaultBodyLimit::max(64 * 1024 * 1024)),
        ] {
            let app = Router::new().nest("/scim", scim.clone()).layer(outer_limit);

            let (status, _, _) = send(&app, "POST", "/scim/Groups", oversized_group.clone()).await;
            assert_eq!(
                status,
                StatusCode::PAYLOAD_TOO_LARGE,
                "POST /Groups, outer limit {outer_name}"
            );
            let (status, _, _) =
                send(&app, "PATCH", "/scim/Groups/g-1", oversized_patch.clone()).await;
            assert_eq!(
                status,
                StatusCode::PAYLOAD_TOO_LARGE,
                "PATCH /Groups/{{id}}, outer limit {outer_name}"
            );

            // Bodies within the limit still reach the handlers.
            let (status, _, _) = send(&app, "POST", "/scim/Groups", group.clone()).await;
            assert_eq!(
                status,
                StatusCode::OK,
                "POST /Groups, outer limit {outer_name}"
            );
            let (status, _, _) = send(&app, "PATCH", "/scim/Groups/g-1", patch.clone()).await;
            assert_eq!(
                status,
                StatusCode::OK,
                "PATCH /Groups/{{id}}, outer limit {outer_name}"
            );
        }
    }

    #[tokio::test]
    async fn group_member_cap_rejects_create_replace_and_patch_over_the_cap() {
        let (_temp, service) = test_service_with_config(
            vec!["scim.write"],
            ScimConfig::new("okta").with_max_group_members_per_request(3),
        )
        .await;
        let app = Arc::new(service).router();
        let body = |value: serde_json::Value| serde_json::to_vec(&value).unwrap();

        assert_invalid_value(
            "POST /Groups",
            send(
                &app,
                "POST",
                "/Groups",
                body(group_json("g-1", "agents", 4)),
            )
            .await,
        );
        let (status, _, _) = send(
            &app,
            "POST",
            "/Groups",
            body(group_json("g-1", "agents", 3)),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "POST /Groups at the cap");

        assert_invalid_value(
            "PUT /Groups/{id}",
            send(
                &app,
                "PUT",
                "/Groups/g-1",
                body(group_json("g-1", "agents", 4)),
            )
            .await,
        );
        let (status, _, _) = send(
            &app,
            "PUT",
            "/Groups/g-1",
            body(group_json("g-1", "agents", 3)),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "PUT /Groups/{{id}} at the cap");

        assert_invalid_value(
            "PATCH /Groups/{id}",
            send(&app, "PATCH", "/Groups/g-1", body(members_patch_json(4))).await,
        );
        let (status, _, response) =
            send(&app, "PATCH", "/Groups/g-1", body(members_patch_json(3))).await;
        assert_eq!(status, StatusCode::OK, "PATCH /Groups/{{id}} at the cap");
        let group: serde_json::Value = serde_json::from_slice(&response).unwrap();
        assert_eq!(group["members"].as_array().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn group_member_cap_applies_to_direct_service_calls() {
        let (_temp, service) = test_service_with_config(
            vec!["scim.write"],
            ScimConfig::new("okta").with_max_group_members_per_request(3),
        )
        .await;
        let group = |member_count| ScimGroup {
            schemas: vec![],
            id: Some("g-1".to_string()),
            external_id: None,
            display_name: "agents".to_string(),
            members: members(member_count),
        };
        let replace_members = |member_count| ScimPatchRequest {
            operations: vec![ScimPatchOperation {
                op: "replace".to_string(),
                path: Some("members".to_string()),
                value: serde_json::to_value(members(member_count)).unwrap(),
            }],
        };

        assert!(matches!(
            service
                .create_group_authorized("Bearer admin-token", group(4))
                .await,
            Err(ScimError::InvalidValue(_))
        ));
        assert!(matches!(
            service.create_or_update_group(group(4)).await,
            Err(ScimError::InvalidValue(_))
        ));
        service
            .create_group_authorized("Bearer admin-token", group(3))
            .await
            .unwrap();

        assert!(matches!(
            service
                .patch_group_authorized("Bearer admin-token", "g-1", replace_members(4))
                .await,
            Err(ScimError::InvalidValue(_))
        ));
        assert!(matches!(
            service.patch_group("g-1", replace_members(4)).await,
            Err(ScimError::InvalidValue(_))
        ));
        let patched = service
            .patch_group("g-1", replace_members(2))
            .await
            .unwrap();
        assert_eq!(patched.members.len(), 2);
    }

    /// Errors other than `InvalidValue` keep the plain-text bodies the router
    /// has always sent.
    #[tokio::test]
    async fn other_http_errors_keep_their_plain_text_bodies() {
        let (_temp, service) = test_service(vec!["scim.write"]).await;
        let app = Arc::new(service).router();
        let (status, content_type, body) = send(&app, "GET", "/Groups/missing", Vec::new()).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(content_type.as_deref(), Some("text/plain; charset=utf-8"));
        assert_eq!(body, b"SCIM resource was not found");
    }

    /// axum 0.8 changed path parameters from `/:id` to `/{id}`; the old
    /// syntax panics when the router is built. Every parameterised route must
    /// reach its handler (no 404/405).
    #[tokio::test]
    async fn router_builds_and_matches_every_path_parameter_route() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let (_temp, service) = test_service(vec!["scim"]).await;
        let app = Arc::new(service).router();
        for (method, uri) in [
            ("GET", "/Users/u-1"),
            ("PUT", "/Users/u-1"),
            ("PATCH", "/Users/u-1"),
            ("DELETE", "/Users/u-1"),
            ("GET", "/Groups/g-1"),
            ("PUT", "/Groups/g-1"),
            ("PATCH", "/Groups/g-1"),
            ("DELETE", "/Groups/g-1"),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert!(
                !matches!(
                    response.status(),
                    StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
                ),
                "{method} {uri} did not reach its handler ({})",
                response.status()
            );
        }
    }

    #[test]
    fn provisioning_passwords_always_satisfy_the_users_policy() {
        let validator = PasswordValidator::with_default_policy();
        for _ in 0..64 {
            let password = provisioning_password();
            for username in ["alice", "scim", "abcd", "1234", "qqqq"] {
                validator.validate(&password, username).unwrap();
            }
        }
    }

    #[tokio::test]
    async fn create_user_authorized_provisions_user_and_external_identity() {
        let (_temp, service) = test_service(vec!["scim.write"]).await;
        let user = service
            .create_user_authorized(
                "Bearer admin-token",
                ScimUser {
                    schemas: vec![SCIM_USER_SCHEMA.to_string()],
                    id: None,
                    external_id: Some("okta-1".to_string()),
                    user_name: "alice".to_string(),
                    display_name: Some("Alice Example".to_string()),
                    active: Some(true),
                    emails: Some(vec![ScimEmail {
                        value: "alice@example.test".to_string(),
                        primary: Some(true),
                        type_: Some("work".to_string()),
                    }]),
                    groups: vec![ScimGroupRef {
                        value: "admin".to_string(),
                        display: Some("admin".to_string()),
                    }],
                    meta: None,
                },
            )
            .await
            .unwrap();

        let id = user.id.clone().unwrap();
        let stored = service
            .auth_service
            .user_store()
            .get_user(&id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.username, "alice");
        assert!(stored.roles.iter().any(|role| role == "admin"));

        let link = service
            .auth_service
            .enterprise_identity_store()
            .unwrap()
            .get_external_identity("okta", "okta-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(link.user_id, id);
    }

    #[tokio::test]
    async fn write_scope_can_read_but_missing_write_scope_cannot_create() {
        let (_temp, write_service) = test_service(vec!["scim.write"]).await;
        write_service
            .authorize("Bearer admin-token", "scim.read")
            .await
            .unwrap();

        let (_temp, read_service) = test_service(vec!["scim.read"]).await;
        let denied = read_service
            .create_user_authorized(
                "Bearer admin-token",
                ScimUser {
                    schemas: vec![],
                    id: None,
                    external_id: Some("subject".to_string()),
                    user_name: "bob".to_string(),
                    display_name: None,
                    active: Some(true),
                    emails: None,
                    groups: vec![],
                    meta: None,
                },
            )
            .await;
        assert!(matches!(denied, Err(ScimError::Unauthorized(_))));
    }

    #[tokio::test]
    async fn group_crud_uses_scim_authorization() {
        let (_temp, service) = test_service(vec!["scim.write"]).await;
        let group = service
            .create_group_authorized(
                "Bearer admin-token",
                ScimGroup {
                    schemas: vec![],
                    id: Some("group-1".to_string()),
                    external_id: Some("external-group-1".to_string()),
                    display_name: "supervisors".to_string(),
                    members: vec![ScimGroupRef {
                        value: "alice".to_string(),
                        display: Some("Alice".to_string()),
                    }],
                },
            )
            .await
            .unwrap();
        assert_eq!(group.id.as_deref(), Some("group-1"));

        let listed = service
            .list_groups_authorized(
                "Bearer admin-token",
                ScimListRequest {
                    filter: Some("externalId eq \"external-group-1\"".to_string()),
                    ..ScimListRequest::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(listed.resources.len(), 1);

        let patched = service
            .patch_group_authorized(
                "Bearer admin-token",
                "group-1",
                ScimPatchRequest {
                    operations: vec![ScimPatchOperation {
                        op: "replace".to_string(),
                        path: Some("displayName".to_string()),
                        value: serde_json::json!("tier1-supervisors"),
                    }],
                },
            )
            .await
            .unwrap();
        assert_eq!(patched.display_name, "tier1-supervisors");

        service
            .delete_group_authorized("Bearer admin-token", "group-1")
            .await
            .unwrap();
        assert!(matches!(
            service.get_group("group-1").await,
            Err(ScimError::NotFound)
        ));
    }
}
