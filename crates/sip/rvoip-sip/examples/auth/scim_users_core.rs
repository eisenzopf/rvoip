//! SCIM provisioning into users-core example.
//!
//! Run with:
//!
//!   cargo run -p rvoip-sip --example auth_scim_users_core

use std::sync::Arc;

use async_trait::async_trait;
use rvoip_auth_core::{BearerAuthError, BearerValidator};
use rvoip_core_traits::identity::IdentityAssurance;
use rvoip_core_traits::ids::IdentityId;
use rvoip_scim::{ScimConfig, ScimEmail, ScimGroupRef, ScimService, ScimUser};
use tempfile::TempDir;
use users_core::{init, UsersConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    let users = init(UsersConfig {
        database_url: format!(
            "sqlite://{}?mode=rwc",
            temp_dir.path().join("users.db").display()
        ),
        ..UsersConfig::default()
    })
    .await?;

    let scim = ScimService::new(
        Arc::new(users),
        Arc::new(ExampleAdminBearer),
        ScimConfig::new("example-scim"),
    )?;

    let user = scim
        .create_user_authorized(
            "Bearer admin-token",
            ScimUser {
                schemas: vec![],
                id: None,
                external_id: Some("external-alice-1".to_string()),
                user_name: "alice".to_string(),
                display_name: Some("Alice Example".to_string()),
                active: Some(true),
                emails: Some(vec![ScimEmail {
                    value: "alice@example.test".to_string(),
                    primary: Some(true),
                    type_: Some("work".to_string()),
                }]),
                groups: vec![ScimGroupRef {
                    value: "user".to_string(),
                    display: Some("user".to_string()),
                }],
                meta: None,
            },
        )
        .await?;

    println!("provisioned SCIM user: {} -> {:?}", user.user_name, user.id);
    Ok(())
}

struct ExampleAdminBearer;

#[async_trait]
impl BearerValidator for ExampleAdminBearer {
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
            scopes: vec!["scim.write".to_string()],
        })
    }
}
