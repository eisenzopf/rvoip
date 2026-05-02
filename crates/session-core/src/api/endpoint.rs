//! Simplified endpoint API for softphones, PBX accounts, demos, and IVR legs.
//!
//! [`Endpoint`] is the easiest session-core surface to start with. It wraps
//! [`StreamPeer`], keeps the existing [`SessionHandle`] and [`IncomingCall`]
//! types, and adds only the account/profile conveniences that SIP applications
//! usually need first.

#![deny(missing_docs)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::time::Duration;

use rvoip_sip_core::types::uri::{Scheme, Uri};

use crate::api::handle::SessionHandle;
use crate::api::incoming::IncomingCall;
use crate::api::stream_peer::{PeerControl, StreamPeer};
use crate::api::unified::{Config, Registration, RegistrationHandle};
use crate::errors::{Result, SessionError};
use crate::types::Credentials;

/// A simplified SIP endpoint built on top of [`StreamPeer`].
///
/// Use `Endpoint` when an application wants a compact softphone/PBX-account
/// style API without losing access to the underlying stream/control objects.
/// Advanced applications can call [`control`](Self::control) or
/// [`into_stream_peer`](Self::into_stream_peer) and continue with the lower
/// level APIs.
pub struct Endpoint {
    peer: StreamPeer,
    registration: Option<Registration>,
    registration_handle: Option<RegistrationHandle>,
    registrar: Option<String>,
}

impl Endpoint {
    /// Start a new [`EndpointBuilder`].
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> rvoip_session_core::Result<()> {
    /// let endpoint = rvoip_session_core::Endpoint::builder()
    ///     .name("alice")
    ///     .build()
    ///     .await?;
    /// endpoint.shutdown().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn builder() -> EndpointBuilder {
        EndpointBuilder::new()
    }

    /// Register the configured account with its registrar.
    ///
    /// Repeated calls return the existing registration handle. Build the
    /// endpoint with [`EndpointBuilder::account`],
    /// [`EndpointBuilder::password`], and [`EndpointBuilder::registrar`] or
    /// with [`EndpointBuilder::endpoint_account`] before calling this method.
    pub async fn register(&mut self) -> Result<RegistrationHandle> {
        if let Some(handle) = &self.registration_handle {
            return Ok(handle.clone());
        }

        let registration = self.registration.clone().ok_or_else(|| {
            SessionError::ConfigError(
                "Endpoint has no complete registration account; set account, password, and registrar"
                    .to_string(),
            )
        })?;
        let handle = self.peer.register_with(registration).await?;
        self.registration_handle = Some(handle.clone());
        Ok(handle)
    }

    /// Unregister the current account if it has been registered.
    ///
    /// Calling this on an endpoint that has not registered is a no-op.
    pub async fn unregister(&mut self) -> Result<()> {
        if let Some(handle) = self.registration_handle.take() {
            self.peer.unregister(&handle).await?;
        }
        Ok(())
    }

    /// Initiate an outgoing call and return its [`SessionHandle`].
    ///
    /// Full `sip:` and `sips:` URIs are used unchanged. Bare extensions such
    /// as `"1002"` are resolved through the endpoint registrar when one is
    /// configured.
    pub async fn call(&self, target: &str) -> Result<SessionHandle> {
        let target = self.resolve_target(target)?;
        self.peer.control().call(&target).await
    }

    /// Initiate an outgoing call and wait for it to answer.
    pub async fn call_and_wait(
        &self,
        target: &str,
        timeout: Option<Duration>,
    ) -> Result<SessionHandle> {
        let call = self.call(target).await?;
        call.wait_for_answered(timeout).await
    }

    /// Wait for the next incoming call.
    pub async fn wait_for_incoming(&mut self) -> Result<IncomingCall> {
        self.peer.wait_for_incoming().await
    }

    /// Access the command half of the wrapped [`StreamPeer`].
    pub fn control(&self) -> &PeerControl {
        self.peer.control()
    }

    /// Resolve a dial target the same way [`call`](Self::call) does.
    ///
    /// This is useful for logging or for handing the resolved URI to a lower
    /// level API.
    pub fn resolve_target(&self, target: &str) -> Result<String> {
        normalize_target(self.registrar.as_deref(), target)
    }

    /// Consume this endpoint and return the wrapped [`StreamPeer`].
    pub fn into_stream_peer(self) -> StreamPeer {
        self.peer
    }

    /// Gracefully unregister and shut down the endpoint.
    pub async fn shutdown(self) -> Result<()> {
        self.peer.shutdown().await
    }
}

/// Account information used by [`EndpointBuilder`].
///
/// `EndpointAccount` describes the SIP registrar credentials and optional
/// identity overrides. It maps directly to [`Registration`] plus the default
/// INVITE digest credentials stored on [`Config`].
#[derive(Debug, Clone)]
pub struct EndpointAccount {
    /// SIP URI of the registrar, for example `sip:pbx.example.com` or
    /// `sips:pbx.example.com:5061`.
    pub registrar: String,
    /// Address-of-record user, usually the extension or SIP username.
    pub username: String,
    /// Optional digest-auth username when it differs from [`username`](Self::username).
    pub auth_username: Option<String>,
    /// Digest-auth password.
    pub password: String,
    /// Registration expiry in seconds.
    pub expires: u32,
    /// Optional From/AoR URI override.
    pub from_uri: Option<String>,
    /// Optional Contact URI override.
    pub contact_uri: Option<String>,
}

impl EndpointAccount {
    /// Create a complete endpoint account.
    ///
    /// # Examples
    ///
    /// ```
    /// let account = rvoip_session_core::EndpointAccount::new(
    ///     "sip:pbx.example.com",
    ///     "1001",
    ///     "secret",
    /// );
    /// assert_eq!(account.expires, 3600);
    /// ```
    pub fn new(
        registrar: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            registrar: registrar.into(),
            username: username.into(),
            auth_username: None,
            password: password.into(),
            expires: 3600,
            from_uri: None,
            contact_uri: None,
        }
    }

    /// Set the digest-auth username.
    pub fn auth_username(mut self, username: impl Into<String>) -> Self {
        self.auth_username = Some(username.into());
        self
    }

    /// Set the registration expiry in seconds.
    pub fn expires(mut self, seconds: u32) -> Self {
        self.expires = seconds;
        self
    }

    /// Override the SIP From/AoR URI.
    pub fn from_uri(mut self, uri: impl Into<String>) -> Self {
        self.from_uri = Some(uri.into());
        self
    }

    /// Override the SIP Contact URI.
    pub fn contact_uri(mut self, uri: impl Into<String>) -> Self {
        self.contact_uri = Some(uri.into());
        self
    }
}

/// Deployment profile used by [`EndpointBuilder`].
///
/// These variants intentionally mirror the existing [`Config`] profile
/// constructors so `Endpoint` remains a convenience layer, not a second SIP
/// configuration system.
#[derive(Debug, Clone)]
pub enum EndpointProfile {
    /// Local loopback development profile.
    Local,
    /// Directly reachable LAN PBX endpoint.
    LanPbx,
    /// Asterisk TLS + mandatory SDES-SRTP with symmetric registered-flow reuse.
    AsteriskTlsSrtpRegisteredFlow,
    /// FreeSWITCH/Sofia internal LAN profile.
    FreeSwitchInternal,
    /// FreeSWITCH TLS + mandatory SDES-SRTP with a directly reachable TLS Contact.
    FreeSwitchTlsSrtpReachableContact,
    /// Carrier/SBC style TLS registered-flow operation with outbound proxy.
    CarrierSbc,
    /// Fully custom config; builder account and registration conveniences still apply.
    Custom(Config),
}

impl Default for EndpointProfile {
    fn default() -> Self {
        Self::Local
    }
}

/// Builder for [`Endpoint`].
///
/// The builder first selects a deployment profile, then applies account,
/// registration, media-port, and custom configuration overrides before
/// starting the wrapped [`StreamPeer`].
pub struct EndpointBuilder {
    name: Option<String>,
    profile: EndpointProfile,
    bind_addr: Option<SocketAddr>,
    advertised_addr: Option<SocketAddr>,
    tls_bind_addr: Option<SocketAddr>,
    tls_cert_path: Option<std::path::PathBuf>,
    tls_key_path: Option<std::path::PathBuf>,
    media_port_start: Option<u16>,
    media_port_end: Option<u16>,
    media_public_addr: Option<SocketAddr>,
    outbound_proxy_uri: Option<String>,
    sip_instance: Option<String>,
    account_username: Option<String>,
    auth_username: Option<String>,
    password: Option<String>,
    registrar: Option<String>,
    expires: u32,
    from_uri: Option<String>,
    contact_uri: Option<String>,
    configurators: Vec<Box<dyn FnOnce(&mut Config) + Send>>,
}

impl EndpointBuilder {
    /// Create a builder with the local profile.
    pub fn new() -> Self {
        Self {
            name: None,
            profile: EndpointProfile::Local,
            bind_addr: None,
            advertised_addr: None,
            tls_bind_addr: None,
            tls_cert_path: None,
            tls_key_path: None,
            media_port_start: None,
            media_port_end: None,
            media_public_addr: None,
            outbound_proxy_uri: None,
            sip_instance: None,
            account_username: None,
            auth_username: None,
            password: None,
            registrar: None,
            expires: 3600,
            from_uri: None,
            contact_uri: None,
            configurators: Vec::new(),
        }
    }

    /// Set the display/configuration name.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the SIP account username or extension.
    pub fn account(mut self, username: impl Into<String>) -> Self {
        self.account_username = Some(username.into());
        self
    }

    /// Set all account fields at once.
    pub fn endpoint_account(mut self, account: EndpointAccount) -> Self {
        self.registrar = Some(account.registrar);
        self.account_username = Some(account.username);
        self.auth_username = account.auth_username;
        self.password = Some(account.password);
        self.expires = account.expires;
        self.from_uri = account.from_uri;
        self.contact_uri = account.contact_uri;
        self
    }

    /// Set the digest-auth username when it differs from the account username.
    pub fn auth_username(mut self, username: impl Into<String>) -> Self {
        self.auth_username = Some(username.into());
        self
    }

    /// Set the digest-auth password.
    pub fn password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Set the SIP registrar URI.
    pub fn registrar(mut self, registrar: impl Into<String>) -> Self {
        self.registrar = Some(registrar.into());
        self
    }

    /// Set the registration expiry in seconds.
    pub fn expires(mut self, seconds: u32) -> Self {
        self.expires = seconds;
        self
    }

    /// Select a deployment profile.
    pub fn profile(mut self, profile: EndpointProfile) -> Self {
        self.profile = profile;
        self
    }

    /// Set the SIP bind address.
    pub fn bind_addr(mut self, addr: SocketAddr) -> Self {
        self.bind_addr = Some(addr);
        self
    }

    /// Set the SIP advertised/public address.
    pub fn advertised_addr(mut self, addr: SocketAddr) -> Self {
        self.advertised_addr = Some(addr);
        self
    }

    /// Set the SIP TLS listener bind address.
    pub fn tls_bind_addr(mut self, addr: SocketAddr) -> Self {
        self.tls_bind_addr = Some(addr);
        self
    }

    /// Set the TLS listener certificate path.
    pub fn tls_cert_path(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.tls_cert_path = Some(path.into());
        self
    }

    /// Set the TLS listener private-key path.
    pub fn tls_key_path(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.tls_key_path = Some(path.into());
        self
    }

    /// Set the RTP media port range.
    pub fn media_ports(mut self, start: u16, end: u16) -> Self {
        self.media_port_start = Some(start);
        self.media_port_end = Some(end);
        self
    }

    /// Set the public RTP media address advertised in SDP.
    pub fn media_public_addr(mut self, addr: SocketAddr) -> Self {
        self.media_public_addr = Some(addr);
        self
    }

    /// Set an outbound proxy URI for carrier/SBC-style operation.
    pub fn outbound_proxy(mut self, uri: impl Into<String>) -> Self {
        self.outbound_proxy_uri = Some(uri.into());
        self
    }

    /// Set the RFC 5626 SIP instance URN used by registered-flow profiles.
    pub fn sip_instance(mut self, urn: impl Into<String>) -> Self {
        self.sip_instance = Some(urn.into());
        self
    }

    /// Override the From/AoR URI used for registration and outgoing calls.
    pub fn from_uri(mut self, uri: impl Into<String>) -> Self {
        self.from_uri = Some(uri.into());
        self
    }

    /// Override the Contact URI used for registration and dialog Contact generation.
    pub fn contact_uri(mut self, uri: impl Into<String>) -> Self {
        self.contact_uri = Some(uri.into());
        self
    }

    /// Mutate the generated [`Config`] immediately before the endpoint starts.
    pub fn configure(mut self, f: impl FnOnce(&mut Config) + Send + 'static) -> Self {
        self.configurators.push(Box::new(f));
        self
    }

    /// Build and start the endpoint.
    pub async fn build(self) -> Result<Endpoint> {
        let parts = self.build_parts()?;
        let peer = StreamPeer::with_config(parts.config).await?;
        Ok(Endpoint {
            peer,
            registration: parts.registration,
            registration_handle: None,
            registrar: parts.registrar,
        })
    }

    fn build_parts(self) -> Result<EndpointParts> {
        let mut config = self.profile_config()?;
        let registrar = self.registrar.clone();
        let account_username = self.account_username.clone();

        if let (Some(username), Some(password)) = (&account_username, &self.password) {
            let auth_username = self.auth_username.as_deref().unwrap_or(username);
            config.credentials = Some(Credentials::new(auth_username, password));
        }

        if let Some(start) = self.media_port_start {
            config.media_port_start = start;
        }
        if let Some(end) = self.media_port_end {
            config.media_port_end = end;
        }
        if let Some(addr) = self.media_public_addr {
            config.media_public_addr = Some(addr);
        }

        let derived_from_uri = match (&self.from_uri, &account_username, &registrar) {
            (Some(uri), _, _) => Some(uri.clone()),
            (None, Some(username), Some(registrar)) => Some(account_aor_uri(registrar, username)?),
            _ => None,
        };
        if let Some(from_uri) = &derived_from_uri {
            config.local_uri = from_uri.clone();
        }

        if let Some(contact_uri) = &self.contact_uri {
            config.contact_uri = Some(contact_uri.clone());
        }

        for configure in self.configurators {
            configure(&mut config);
        }

        let registration = match (
            registrar.as_ref(),
            account_username.as_ref(),
            self.password.as_ref(),
        ) {
            (Some(registrar), Some(username), Some(password)) => {
                let auth_username = self.auth_username.as_deref().unwrap_or(username);
                let mut registration = Registration::new(
                    registrar.clone(),
                    auth_username.to_string(),
                    password.clone(),
                )
                .expires(self.expires);
                if let Some(from_uri) = derived_from_uri {
                    registration = registration.from_uri(from_uri);
                }
                if let Some(contact_uri) = self.contact_uri {
                    registration = registration.contact_uri(contact_uri);
                }
                Some(registration)
            }
            _ => None,
        };

        Ok(EndpointParts {
            config,
            registration,
            registrar,
        })
    }

    fn profile_config(&self) -> Result<Config> {
        let name = self
            .name
            .as_deref()
            .or(self.account_username.as_deref())
            .unwrap_or("endpoint");

        match &self.profile {
            EndpointProfile::Local => {
                let bind = self
                    .bind_addr
                    .unwrap_or_else(|| SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 5060));
                if bind.ip().is_loopback() {
                    Ok(Config::local(name, bind.port()))
                } else {
                    let mut config = Config::on(name, bind.ip(), bind.port());
                    config.bind_addr = bind;
                    Ok(config)
                }
            }
            EndpointProfile::LanPbx => {
                let bind = self.bind_addr.unwrap_or_else(default_udp_bind);
                let advertised = self.advertised_addr.ok_or_else(|| {
                    SessionError::ConfigError(
                        "EndpointProfile::LanPbx requires advertised_addr".to_string(),
                    )
                })?;
                Ok(Config::lan_pbx(name, bind, advertised))
            }
            EndpointProfile::AsteriskTlsSrtpRegisteredFlow => {
                let bind = self.bind_addr.unwrap_or_else(default_tls_bind);
                Ok(Config::asterisk_tls_registered_flow(
                    name,
                    bind,
                    self.sip_instance
                        .clone()
                        .unwrap_or_else(generate_sip_instance),
                ))
            }
            EndpointProfile::FreeSwitchInternal => {
                let bind = self.bind_addr.unwrap_or_else(default_udp_bind);
                Ok(Config::freeswitch_internal(name, bind))
            }
            EndpointProfile::FreeSwitchTlsSrtpReachableContact => {
                let bind = self.bind_addr.unwrap_or_else(default_udp_bind);
                let tls_bind = self.tls_bind_addr.unwrap_or_else(default_tls_bind);
                let cert = self.tls_cert_path.clone().ok_or_else(|| {
                    SessionError::ConfigError(
                        "EndpointProfile::FreeSwitchTlsSrtpReachableContact requires tls_cert_path"
                            .to_string(),
                    )
                })?;
                let key = self.tls_key_path.clone().ok_or_else(|| {
                    SessionError::ConfigError(
                        "EndpointProfile::FreeSwitchTlsSrtpReachableContact requires tls_key_path"
                            .to_string(),
                    )
                })?;
                Ok(Config::freeswitch_tls_srtp_reachable_contact(
                    name, bind, tls_bind, cert, key,
                ))
            }
            EndpointProfile::CarrierSbc => {
                let bind = self.bind_addr.unwrap_or_else(default_tls_bind);
                let public = self.advertised_addr.ok_or_else(|| {
                    SessionError::ConfigError(
                        "EndpointProfile::CarrierSbc requires advertised_addr".to_string(),
                    )
                })?;
                let outbound_proxy = self.outbound_proxy_uri.clone().ok_or_else(|| {
                    SessionError::ConfigError(
                        "EndpointProfile::CarrierSbc requires outbound_proxy".to_string(),
                    )
                })?;
                Ok(Config::carrier_sbc(
                    name,
                    bind,
                    public,
                    outbound_proxy,
                    self.sip_instance
                        .clone()
                        .unwrap_or_else(generate_sip_instance),
                ))
            }
            EndpointProfile::Custom(config) => Ok(config.clone()),
        }
    }
}

impl Default for EndpointBuilder {
    fn default() -> Self {
        Self::new()
    }
}

struct EndpointParts {
    config: Config,
    registration: Option<Registration>,
    registrar: Option<String>,
}

fn default_udp_bind() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 5060)
}

fn default_tls_bind() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 5061)
}

fn generate_sip_instance() -> String {
    format!("urn:uuid:{}", uuid::Uuid::new_v4())
}

fn normalize_target(registrar: Option<&str>, target: &str) -> Result<String> {
    let target = target.trim();
    if target.is_empty() {
        return Err(SessionError::InvalidInput(
            "call target must not be empty".to_string(),
        ));
    }

    let lower = target.to_ascii_lowercase();
    if lower.starts_with("sip:") || lower.starts_with("sips:") || lower.starts_with("tel:") {
        return Ok(target.to_string());
    }

    let registrar = registrar.ok_or_else(|| {
        SessionError::ConfigError(
            "bare call targets require EndpointBuilder::registrar".to_string(),
        )
    })?;
    let mut registrar_uri = parse_uri(registrar, "registrar")?;

    if target.contains('@') {
        return Ok(format!("{}:{}", registrar_uri.scheme, target));
    }

    registrar_uri.user = Some(target.to_string());
    registrar_uri.password = None;
    registrar_uri.headers.clear();
    Ok(registrar_uri.to_string())
}

fn account_aor_uri(registrar: &str, username: &str) -> Result<String> {
    let mut uri = parse_uri(registrar, "registrar")?;
    uri.user = Some(username.to_string());
    uri.password = None;
    uri.port = None;
    uri.parameters.clear();
    uri.headers.clear();
    Ok(uri.to_string())
}

fn parse_uri(value: &str, label: &str) -> Result<Uri> {
    let uri = Uri::from_str(value).map_err(|err| {
        SessionError::InvalidInput(format!("invalid {label} URI '{value}': {err}"))
    })?;
    match uri.scheme {
        Scheme::Sip | Scheme::Sips => Ok(uri),
        _ => Err(SessionError::InvalidInput(format!(
            "{label} URI must use sip: or sips:"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::unified::{SipContactMode, SipTlsMode};

    #[test]
    fn endpoint_builder_maps_asterisk_tls_profile() {
        let parts = Endpoint::builder()
            .name("alice")
            .account("1001")
            .password("secret")
            .registrar("sips:pbx.example.test:5061;transport=tls")
            .profile(EndpointProfile::AsteriskTlsSrtpRegisteredFlow)
            .sip_instance("urn:uuid:00000000-0000-0000-0000-000000000001")
            .build_parts()
            .unwrap();

        assert_eq!(parts.config.sip_tls_mode, SipTlsMode::ClientOnly);
        assert_eq!(
            parts.config.sip_contact_mode,
            SipContactMode::RegisteredFlowSymmetric
        );
        assert!(parts.config.offer_srtp);
        assert!(parts.config.srtp_required);
        assert_eq!(parts.config.local_uri, "sips:1001@pbx.example.test");
        assert!(parts.registration.is_some());
    }

    #[test]
    fn endpoint_builder_creates_registration_defaults() {
        let parts = Endpoint::builder()
            .account("1001")
            .auth_username("auth1001")
            .password("secret")
            .registrar("sip:pbx.example.test")
            .contact_uri("sip:1001@192.0.2.10:5060")
            .expires(600)
            .build_parts()
            .unwrap();

        let registration = parts.registration.unwrap();
        assert_eq!(registration.registrar, "sip:pbx.example.test");
        assert_eq!(registration.username, "auth1001");
        assert_eq!(registration.password, "secret");
        assert_eq!(registration.expires, 600);
        assert_eq!(
            registration.from_uri.as_deref(),
            Some("sip:1001@pbx.example.test")
        );
        assert_eq!(
            registration.contact_uri.as_deref(),
            Some("sip:1001@192.0.2.10:5060")
        );
    }

    #[test]
    fn endpoint_normalizes_bare_extension_through_registrar() {
        let target =
            normalize_target(Some("sips:pbx.example.test:5061;transport=tls"), "1002").unwrap();
        assert_eq!(target, "sips:1002@pbx.example.test:5061;transport=tls");
    }

    #[test]
    fn endpoint_leaves_full_sip_uri_unchanged() {
        let target =
            normalize_target(Some("sips:pbx.example.test:5061"), "sip:bob@example.test").unwrap();
        assert_eq!(target, "sip:bob@example.test");
    }

    #[test]
    fn endpoint_requires_registrar_for_bare_target() {
        let err = normalize_target(None, "1002").unwrap_err();
        assert!(err.to_string().contains("registrar"));
    }
}
