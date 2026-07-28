//! LDAP-backed password verifier for RVoIP auth providers.
//!
//! This crate implements `rvoip-auth-core::PasswordVerifier` for directories
//! that support LDAP simple bind. Use it for legacy Basic-over-TLS deployments
//! or other password verification paths where LDAP/AD owns the credential.

use async_trait::async_trait;
use ldap3::{LdapConnAsync, Scope, SearchEntry};
use rvoip_auth_core::{CredentialAuthError, PasswordVerifier};
use rvoip_core_traits::identity::IdentityAssurance;
use rvoip_core_traits::ids::IdentityId;
use std::fmt;

/// LDAP verifier construction/configuration error.
pub enum LdapVerifierError {
    /// Required config was missing.
    Config(String),
}

impl fmt::Display for LdapVerifierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LDAP verifier failed (class=configuration)")
    }
}

impl fmt::Debug for LdapVerifierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl std::error::Error for LdapVerifierError {}

/// Known LDAP directory filter presets.
///
/// LDAP remains a compatibility/password-verification provider. New
/// applications should prefer OIDC/SAML for login and SCIM for lifecycle
/// provisioning, with LDAP/AD/389DS/FreeIPA usually sitting behind the IdP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LdapDirectoryPreset {
    /// OpenLDAP-style users with `uid`.
    OpenLdap,
    /// 389 Directory Server-style users with `uid` and `mail`.
    Directory389,
    /// FreeIPA users with `uid`, `mail`, or Kerberos principal.
    FreeIpa,
    /// Active Directory-compatible lookup by UPN, sAMAccountName, or mail.
    ActiveDirectory,
}

impl LdapDirectoryPreset {
    pub fn user_filter_template(self) -> &'static str {
        match self {
            Self::OpenLdap => "(uid={username})",
            Self::Directory389 => "(|(uid={username})(mail={username}))",
            Self::FreeIpa => "(|(uid={username})(mail={username})(krbPrincipalName={username}))",
            Self::ActiveDirectory => {
                "(|(userPrincipalName={username})(sAMAccountName={username})(mail={username}))"
            }
        }
    }

    pub fn scope(self) -> &'static str {
        match self {
            Self::OpenLdap => "ldap.openldap",
            Self::Directory389 => "ldap.389ds",
            Self::FreeIpa => "ldap.freeipa",
            Self::ActiveDirectory => "ldap.ad",
        }
    }
}

/// LDAP password verifier configuration.
#[derive(Clone)]
pub struct LdapPasswordVerifierConfig {
    /// LDAP URL, for example `ldap://127.0.0.1:1389` or `ldaps://...`.
    pub url: String,
    /// Optional service bind DN used before searching for users.
    pub bind_dn: Option<String>,
    /// Optional service bind password.
    pub bind_password: Option<String>,
    /// Base DN for user searches.
    pub user_base_dn: String,
    /// LDAP search filter template. `{username}` and `{user}` are replaced
    /// with an RFC 4515-escaped username.
    pub user_filter_template: String,
    /// Scopes returned in `IdentityAssurance::UserAuthorized` on success.
    pub scopes: Vec<String>,
}

impl fmt::Debug for LdapPasswordVerifierConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LdapPasswordVerifierConfig")
            .field("url_present", &!self.url.is_empty())
            .field("bind_dn_present", &self.bind_dn.is_some())
            .field("bind_dn_len", &self.bind_dn.as_deref().map(str::len))
            .field("bind_password_present", &self.bind_password.is_some())
            .field(
                "bind_password_len",
                &self.bind_password.as_deref().map(str::len),
            )
            .field("user_base_dn_present", &!self.user_base_dn.is_empty())
            .field("user_filter_template_len", &self.user_filter_template.len())
            .field("scope_count", &self.scopes.len())
            .finish()
    }
}

impl LdapPasswordVerifierConfig {
    /// Create a config with the common OpenLDAP `uid` user filter.
    pub fn new(url: impl Into<String>, user_base_dn: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            bind_dn: None,
            bind_password: None,
            user_base_dn: user_base_dn.into(),
            user_filter_template: "(uid={username})".to_string(),
            scopes: vec!["sip.basic".to_string()],
        }
    }

    /// Create config for a known directory preset.
    pub fn for_preset(
        url: impl Into<String>,
        user_base_dn: impl Into<String>,
        preset: LdapDirectoryPreset,
    ) -> Self {
        Self::new(url, user_base_dn).with_preset(preset)
    }

    /// Create OpenLDAP config with `uid={username}` lookup.
    pub fn openldap(url: impl Into<String>, user_base_dn: impl Into<String>) -> Self {
        Self::for_preset(url, user_base_dn, LdapDirectoryPreset::OpenLdap)
    }

    /// Create 389 Directory Server config with `uid`/`mail` lookup.
    pub fn directory_389(url: impl Into<String>, user_base_dn: impl Into<String>) -> Self {
        Self::for_preset(url, user_base_dn, LdapDirectoryPreset::Directory389)
    }

    /// Create FreeIPA config with `uid`/`mail`/`krbPrincipalName` lookup.
    pub fn freeipa(url: impl Into<String>, user_base_dn: impl Into<String>) -> Self {
        Self::for_preset(url, user_base_dn, LdapDirectoryPreset::FreeIpa)
    }

    /// Create Active Directory-compatible config with UPN/sAM/mail lookup.
    pub fn active_directory(url: impl Into<String>, user_base_dn: impl Into<String>) -> Self {
        Self::for_preset(url, user_base_dn, LdapDirectoryPreset::ActiveDirectory)
    }

    /// Configure service bind credentials used for user search.
    pub fn with_bind_credentials(
        mut self,
        bind_dn: impl Into<String>,
        bind_password: impl Into<String>,
    ) -> Self {
        self.bind_dn = Some(bind_dn.into());
        self.bind_password = Some(bind_password.into());
        self
    }

    /// Configure an LDAP user search filter template.
    pub fn with_user_filter_template(mut self, template: impl Into<String>) -> Self {
        self.user_filter_template = template.into();
        self
    }

    /// Apply a known directory filter preset and add the preset scope.
    pub fn with_preset(mut self, preset: LdapDirectoryPreset) -> Self {
        self.user_filter_template = preset.user_filter_template().to_string();
        if !self.scopes.iter().any(|scope| scope == preset.scope()) {
            self.scopes.push(preset.scope().to_string());
        }
        self
    }

    /// Configure scopes returned on successful verification.
    pub fn with_scopes(mut self, scopes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.scopes = scopes.into_iter().map(Into::into).collect();
        self
    }
}

/// LDAP-backed password verifier.
#[derive(Clone)]
pub struct LdapPasswordVerifier {
    config: LdapPasswordVerifierConfig,
}

impl fmt::Debug for LdapPasswordVerifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LdapPasswordVerifier")
            .field("config", &self.config)
            .finish()
    }
}

impl LdapPasswordVerifier {
    /// Create an LDAP password verifier.
    pub fn new(config: LdapPasswordVerifierConfig) -> Result<Self, LdapVerifierError> {
        if config.url.trim().is_empty() {
            return Err(LdapVerifierError::Config(
                "LDAP URL is required".to_string(),
            ));
        }
        if config.user_base_dn.trim().is_empty() {
            return Err(LdapVerifierError::Config(
                "LDAP user_base_dn is required".to_string(),
            ));
        }
        if config.bind_dn.is_some() != config.bind_password.is_some() {
            return Err(LdapVerifierError::Config(
                "LDAP bind_dn and bind_password must be configured together".to_string(),
            ));
        }
        Ok(Self { config })
    }

    /// Return verifier configuration.
    pub fn config(&self) -> &LdapPasswordVerifierConfig {
        &self.config
    }

    async fn find_user_dn(&self, username: &str) -> Result<String, CredentialAuthError> {
        let (conn, mut ldap) = LdapConnAsync::new(&self.config.url)
            .await
            .map_ldap_unavailable()?;
        ldap3::drive!(conn);

        if let (Some(bind_dn), Some(bind_password)) = (
            self.config.bind_dn.as_ref(),
            self.config.bind_password.as_ref(),
        ) {
            ldap.simple_bind(bind_dn, bind_password)
                .await
                .map_ldap_unavailable()?
                .success()
                .map_ldap_unavailable()?;
        }

        let filter = render_user_filter(&self.config.user_filter_template, username);
        let (entries, _result) = ldap
            .search(
                &self.config.user_base_dn,
                Scope::Subtree,
                &filter,
                vec!["dn"],
            )
            .await
            .map_ldap_unavailable()?
            .success()
            .map_ldap_unavailable()?;
        let mut dns = entries
            .into_iter()
            .map(SearchEntry::construct)
            .map(|entry| entry.dn)
            .collect::<Vec<_>>();

        let _ = ldap.unbind().await;

        match dns.len() {
            1 => Ok(dns.remove(0)),
            0 => Err(CredentialAuthError::Invalid),
            _ => Err(CredentialAuthError::PolicyRejected(
                "LDAP user search returned multiple entries".to_string(),
            )),
        }
    }
}

#[async_trait]
impl PasswordVerifier for LdapPasswordVerifier {
    async fn verify_password(
        &self,
        username: &str,
        password: &str,
    ) -> Result<IdentityAssurance, CredentialAuthError> {
        if username.trim().is_empty() || password.is_empty() {
            return Err(CredentialAuthError::Invalid);
        }

        let user_dn = self.find_user_dn(username).await?;
        let (conn, mut ldap) = LdapConnAsync::new(&self.config.url)
            .await
            .map_ldap_unavailable()?;
        ldap3::drive!(conn);
        ldap.simple_bind(&user_dn, password)
            .await
            .map_ldap_unavailable()?
            .success()
            .map_ldap_invalid()?;
        let _ = ldap.unbind().await;

        let identity = IdentityId::from_string(format!("ldap:{username}"));
        Ok(IdentityAssurance::UserAuthorized {
            identity: identity.clone(),
            user_id: identity,
            scopes: self.config.scopes.clone(),
        })
    }
}

trait LdapResultExt<T> {
    fn map_ldap_unavailable(self) -> Result<T, CredentialAuthError>;
    fn map_ldap_invalid(self) -> Result<T, CredentialAuthError>;
}

impl<T, E> LdapResultExt<T> for Result<T, E>
where
    E: std::fmt::Display,
{
    fn map_ldap_unavailable(self) -> Result<T, CredentialAuthError> {
        self.map_err(|err| CredentialAuthError::Unavailable(err.to_string()))
    }

    fn map_ldap_invalid(self) -> Result<T, CredentialAuthError> {
        self.map_err(|_| CredentialAuthError::Invalid)
    }
}

fn render_user_filter(template: &str, username: &str) -> String {
    let escaped = escape_filter_value(username);
    template
        .replace("{username}", &escaped)
        .replace("{user}", &escaped)
}

fn escape_filter_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '*' => escaped.push_str(r"\2a"),
            '(' => escaped.push_str(r"\28"),
            ')' => escaped.push_str(r"\29"),
            '\\' => escaped.push_str(r"\5c"),
            '\0' => escaped.push_str(r"\00"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ldap_filter_values_are_escaped() {
        assert_eq!(
            render_user_filter("(|(uid={username})(mail={user}))", r"a*b(c)d\e\0"),
            r"(|(uid=a\2ab\28c\29d\5ce\5c0)(mail=a\2ab\28c\29d\5ce\5c0))"
        );
    }

    #[test]
    fn config_rejects_partial_bind_credentials() {
        let error = LdapPasswordVerifier::new(LdapPasswordVerifierConfig {
            bind_dn: Some("cn=admin,dc=example,dc=test".to_string()),
            bind_password: None,
            ..LdapPasswordVerifierConfig::new(
                "ldap://127.0.0.1:1389",
                "ou=users,dc=example,dc=test",
            )
        })
        .expect_err("partial bind credentials should fail");
        assert_eq!(
            error.to_string(),
            "LDAP verifier failed (class=configuration)"
        );
        match error {
            LdapVerifierError::Config(detail) => assert!(detail.contains("configured together")),
        }
    }

    #[test]
    fn directory_presets_configure_expected_filters_and_scopes() {
        let configs = [
            (
                LdapDirectoryPreset::OpenLdap,
                "(uid={username})",
                "ldap.openldap",
            ),
            (
                LdapDirectoryPreset::Directory389,
                "(|(uid={username})(mail={username}))",
                "ldap.389ds",
            ),
            (
                LdapDirectoryPreset::FreeIpa,
                "(|(uid={username})(mail={username})(krbPrincipalName={username}))",
                "ldap.freeipa",
            ),
            (
                LdapDirectoryPreset::ActiveDirectory,
                "(|(userPrincipalName={username})(sAMAccountName={username})(mail={username}))",
                "ldap.ad",
            ),
        ];

        for (preset, filter, scope) in configs {
            let config = LdapPasswordVerifierConfig::for_preset(
                "ldap://127.0.0.1:1389",
                "dc=example,dc=test",
                preset,
            );
            assert_eq!(config.user_filter_template, filter);
            assert!(config.scopes.iter().any(|configured| configured == scope));
        }
    }

    #[test]
    fn bind_config_verifier_and_errors_redact_credentials() {
        const CANARY: &str = "ldap-bind-credential-canary\r\nAuthorization: exposed";
        let config = LdapPasswordVerifierConfig::new(CANARY, CANARY)
            .with_bind_credentials(CANARY, CANARY)
            .with_user_filter_template(CANARY)
            .with_scopes([CANARY]);
        let verifier = LdapPasswordVerifier::new(config.clone()).unwrap();
        let error = LdapVerifierError::Config(CANARY.into());

        for rendered in [
            format!("{config:?}"),
            format!("{verifier:?}"),
            format!("{error} {error:?}"),
        ] {
            assert!(!rendered.contains(CANARY), "credential leaked: {rendered}");
        }
        assert_eq!(config.bind_password.as_deref(), Some(CANARY));
        match error {
            LdapVerifierError::Config(value) => assert_eq!(value, CANARY),
        }
    }

    #[tokio::test]
    async fn live_openldap_verifies_password_when_configured() {
        let Some(url) = std::env::var("RVOIP_LDAP_URL").ok() else {
            return;
        };
        let user_base_dn = std::env::var("RVOIP_LDAP_USER_BASE_DN")
            .unwrap_or_else(|_| "ou=users,dc=rvoip,dc=local".to_string());
        let mut config = LdapPasswordVerifierConfig::new(url, user_base_dn);
        if let (Ok(bind_dn), Ok(bind_password)) = (
            std::env::var("RVOIP_LDAP_BIND_DN"),
            std::env::var("RVOIP_LDAP_BIND_PASSWORD"),
        ) {
            config = config.with_bind_credentials(bind_dn, bind_password);
        }
        let verifier = LdapPasswordVerifier::new(config).unwrap();

        let assurance = verifier
            .verify_password("alice", "alicepass")
            .await
            .expect("alice should authenticate against live LDAP fixture");
        assert!(matches!(
            assurance,
            IdentityAssurance::UserAuthorized { .. }
        ));
        assert!(matches!(
            verifier.verify_password("alice", "wrong").await,
            Err(CredentialAuthError::Invalid)
        ));
    }

    #[tokio::test]
    async fn live_active_directory_verifies_password_when_configured() {
        let Some(url) = std::env::var("RVOIP_AD_LDAP_URL").ok() else {
            return;
        };
        let user_base_dn = std::env::var("RVOIP_AD_USER_BASE_DN")
            .unwrap_or_else(|_| "CN=Users,DC=rvoip,DC=local".to_string());
        let username =
            std::env::var("RVOIP_AD_TEST_USERNAME").unwrap_or_else(|_| "alice@rvoip.local".into());
        let password =
            std::env::var("RVOIP_AD_TEST_PASSWORD").unwrap_or_else(|_| "alicepass".into());
        let filter = std::env::var("RVOIP_AD_USER_FILTER").unwrap_or_else(|_| {
            "(|(userPrincipalName={username})(sAMAccountName={username}))".into()
        });
        let mut config = LdapPasswordVerifierConfig::new(url, user_base_dn)
            .with_user_filter_template(filter)
            .with_scopes(["sip.basic", "ad.user"]);
        if let (Ok(bind_dn), Ok(bind_password)) = (
            std::env::var("RVOIP_AD_BIND_DN"),
            std::env::var("RVOIP_AD_BIND_PASSWORD"),
        ) {
            config = config.with_bind_credentials(bind_dn, bind_password);
        }
        let verifier = LdapPasswordVerifier::new(config).unwrap();

        let assurance = verifier
            .verify_password(&username, &password)
            .await
            .expect("configured AD-compatible LDAP user should authenticate");
        assert!(matches!(
            assurance,
            IdentityAssurance::UserAuthorized { .. }
        ));
        assert!(matches!(
            verifier.verify_password(&username, "wrong").await,
            Err(CredentialAuthError::Invalid)
        ));
    }

    #[tokio::test]
    async fn live_389ds_verifies_password_when_configured() {
        let Some(url) = std::env::var("RVOIP_389DS_LDAP_URL").ok() else {
            return;
        };
        let user_base_dn = std::env::var("RVOIP_389DS_USER_BASE_DN")
            .unwrap_or_else(|_| "ou=people,dc=rvoip,dc=local".to_string());
        let username =
            std::env::var("RVOIP_389DS_TEST_USERNAME").unwrap_or_else(|_| "alice".into());
        let password =
            std::env::var("RVOIP_389DS_TEST_PASSWORD").unwrap_or_else(|_| "alicepass".into());
        let mut config = LdapPasswordVerifierConfig::directory_389(url, user_base_dn);
        if let (Ok(bind_dn), Ok(bind_password)) = (
            std::env::var("RVOIP_389DS_BIND_DN"),
            std::env::var("RVOIP_389DS_BIND_PASSWORD"),
        ) {
            config = config.with_bind_credentials(bind_dn, bind_password);
        }
        let verifier = LdapPasswordVerifier::new(config).unwrap();

        let assurance = verifier
            .verify_password(&username, &password)
            .await
            .expect("configured 389DS LDAP user should authenticate");
        assert!(matches!(
            assurance,
            IdentityAssurance::UserAuthorized { .. }
        ));
    }

    #[tokio::test]
    async fn live_freeipa_verifies_password_when_configured() {
        let Some(url) = std::env::var("RVOIP_FREEIPA_LDAP_URL").ok() else {
            return;
        };
        let user_base_dn = std::env::var("RVOIP_FREEIPA_USER_BASE_DN")
            .unwrap_or_else(|_| "cn=users,cn=accounts,dc=rvoip,dc=local".to_string());
        let username =
            std::env::var("RVOIP_FREEIPA_TEST_USERNAME").unwrap_or_else(|_| "alice".into());
        let password =
            std::env::var("RVOIP_FREEIPA_TEST_PASSWORD").unwrap_or_else(|_| "alicepass".into());
        let mut config = LdapPasswordVerifierConfig::freeipa(url, user_base_dn);
        if let (Ok(bind_dn), Ok(bind_password)) = (
            std::env::var("RVOIP_FREEIPA_BIND_DN"),
            std::env::var("RVOIP_FREEIPA_BIND_PASSWORD"),
        ) {
            config = config.with_bind_credentials(bind_dn, bind_password);
        }
        let verifier = LdapPasswordVerifier::new(config).unwrap();

        let assurance = verifier
            .verify_password(&username, &password)
            .await
            .expect("configured FreeIPA LDAP user should authenticate");
        assert!(matches!(
            assurance,
            IdentityAssurance::UserAuthorized { .. }
        ));
    }
}
