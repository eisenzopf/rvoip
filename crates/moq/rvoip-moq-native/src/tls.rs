// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use anyhow::Context;
use clap::{Parser, ValueEnum};
use ring::digest::{digest, SHA256};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::{ClientHello, ResolvesServerCert, WebPkiClientVerifier};
use rustls::sign::CertifiedKey;
use rustls::RootCertStore;
use std::fs;
use std::io::{self, Cursor, Read};
use std::path;
use std::sync::Arc;

/// Server policy for TLS client certificates.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum ClientAuthMode {
    /// Do not request a client certificate. This mode is suitable only for
    /// explicitly configured development or for a separate admission scheme.
    #[default]
    Disabled,
    /// Verify a certificate when one is presented, but allow anonymous peers.
    Optional,
    /// Require every peer to present a chain rooted in `--tls-client-ca`.
    Required,
}

#[derive(Parser, Clone, Default)]
#[group(id = "tls")]
pub struct Args {
    /// Use the certificates at this path, encoded as PEM.
    ///
    /// You can use this option multiple times for multiple certificates.
    /// The first match for the provided SNI will be used, otherwise the last cert will be used.
    /// You also need to provide the private key multiple times via `key``.
    #[arg(long = "tls-cert")]
    pub cert: Vec<path::PathBuf>,

    /// Use the private key at this path, encoded as PEM.
    ///
    /// There must be a key for every certificate provided via `cert`.
    #[arg(long = "tls-key")]
    pub key: Vec<path::PathBuf>,

    /// Use the TLS root at this path, encoded as PEM.
    ///
    /// This value can be provided multiple times for multiple roots.
    /// If this is empty, system roots will be used instead
    #[arg(long = "tls-root")]
    pub root: Vec<path::PathBuf>,

    /// Certificate chain presented by this endpoint when acting as a client.
    #[arg(long = "tls-client-cert", requires = "client_key")]
    pub client_cert: Option<path::PathBuf>,

    /// Private key for `--tls-client-cert`.
    #[arg(long = "tls-client-key", requires = "client_cert")]
    pub client_key: Option<path::PathBuf>,

    /// Whether the server requests or requires verified client certificates.
    #[arg(long = "tls-client-auth", value_enum, default_value_t)]
    pub client_auth: ClientAuthMode,

    /// CA certificates trusted for inbound TLS client authentication.
    ///
    /// At least one explicit CA is required for optional or required client
    /// authentication. Platform roots are deliberately never used here.
    #[arg(long = "tls-client-ca")]
    pub client_ca: Vec<path::PathBuf>,

    /// Danger: Disable TLS certificate verification.
    ///
    /// Fine for local development and between relays, but should be used in caution in production.
    #[arg(long = "tls-disable-verify", env = "TLS_DISABLE_VERIFY")]
    pub disable_verify: bool,
}

#[derive(Clone)]
pub struct Config {
    client: rustls::ClientConfig,
    server: Option<rustls::ServerConfig>,
    fingerprints: Vec<String>,
    client_auth: ClientAuthMode,
    verifies_server_certificates: bool,
}

impl Config {
    pub fn has_server(&self) -> bool {
        self.server.is_some()
    }

    pub fn fingerprints(&self) -> &[String] {
        &self.fingerprints
    }

    pub fn client_auth_mode(&self) -> ClientAuthMode {
        self.client_auth
    }

    pub fn verifies_server_certificates(&self) -> bool {
        self.verifies_server_certificates
    }

    /// Consume this bound configuration for the development HTTPS helper.
    /// Returned raw rustls state cannot be converted back into a security
    /// evidence-bearing `Config`.
    pub fn into_https_server_config(self) -> Option<rustls::ServerConfig> {
        self.server
    }

    pub(crate) fn into_quic_parts(self) -> QuicConfigParts {
        QuicConfigParts {
            client: self.client,
            server: self.server,
            client_auth: self.client_auth,
            verifies_server_certificates: self.verifies_server_certificates,
        }
    }
}

pub(crate) struct QuicConfigParts {
    pub client: rustls::ClientConfig,
    pub server: Option<rustls::ServerConfig>,
    pub client_auth: ClientAuthMode,
    pub verifies_server_certificates: bool,
}

impl Args {
    pub fn load(&self) -> anyhow::Result<Config> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut serve = ServeCerts::default();

        // Load the certificate and key files based on their index.
        anyhow::ensure!(
            self.cert.len() == self.key.len(),
            "--tls-cert and --tls-key counts differ"
        );
        for (chain, key) in self.cert.iter().zip(self.key.iter()) {
            serve.load(chain, key)?;
        }

        let roots = load_server_roots(&self.root)?;

        // Create the TLS configuration we'll use as a client (relay -> relay)
        let client = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .with_root_certificates(roots);
        let mut client = match (&self.client_cert, &self.client_key) {
            (Some(chain), Some(key)) => {
                let (chain, key) = load_identity(chain, key, "TLS client identity")?;
                client
                    .with_client_auth_cert(chain, key)
                    .context("invalid TLS client identity")?
            }
            (None, None) => client.with_no_client_auth(),
            _ => anyhow::bail!("--tls-client-cert and --tls-client-key must be provided together"),
        };

        // Allow disabling TLS verification altogether.
        if self.disable_verify {
            let noop = NoCertificateVerification(provider.clone());
            client.dangerous().set_certificate_verifier(Arc::new(noop));
        }

        let fingerprints = serve.fingerprints();

        let client_verifier = match self.client_auth {
            ClientAuthMode::Disabled => {
                anyhow::ensure!(
                    self.client_ca.is_empty(),
                    "--tls-client-ca requires --tls-client-auth optional or required"
                );
                WebPkiClientVerifier::no_client_auth()
            }
            ClientAuthMode::Optional | ClientAuthMode::Required => {
                anyhow::ensure!(
                    !self.client_ca.is_empty(),
                    "--tls-client-auth {:?} requires at least one --tls-client-ca",
                    self.client_auth
                );
                let roots = Arc::new(load_explicit_roots(
                    &self.client_ca,
                    "TLS client CA certificate",
                )?);
                let builder = WebPkiClientVerifier::builder_with_provider(roots, provider.clone());
                if self.client_auth == ClientAuthMode::Optional {
                    builder.allow_unauthenticated().build()?
                } else {
                    builder.build()?
                }
            }
        };

        // Create the TLS configuration we'll use as a server.
        let server = if !self.key.is_empty() {
            Some(
                rustls::ServerConfig::builder_with_provider(provider)
                    .with_protocol_versions(&[&rustls::version::TLS13])?
                    .with_client_cert_verifier(client_verifier)
                    .with_cert_resolver(Arc::new(serve)),
            )
        } else {
            anyhow::ensure!(
                self.client_auth == ClientAuthMode::Disabled,
                "TLS client authentication requires a configured TLS server certificate"
            );
            None
        };

        Ok(Config {
            server,
            client,
            fingerprints,
            client_auth: self.client_auth,
            verifies_server_certificates: !self.disable_verify,
        })
    }
}

fn load_server_roots(paths: &[path::PathBuf]) -> anyhow::Result<RootCertStore> {
    if paths.is_empty() {
        let mut roots = RootCertStore::empty();
        for cert in
            rustls_native_certs::load_native_certs().context("could not load platform certs")?
        {
            roots.add(cert).context("failed to add platform root")?;
        }
        Ok(roots)
    } else {
        load_explicit_roots(paths, "TLS server root certificate")
    }
}

fn load_explicit_roots(
    paths: &[path::PathBuf],
    description: &str,
) -> anyhow::Result<RootCertStore> {
    let mut roots = RootCertStore::empty();
    for path in paths {
        let certificates = load_certificates(path, description)?;
        for certificate in certificates {
            roots
                .add(certificate)
                .with_context(|| format!("failed to add {description}"))?;
        }
    }
    anyhow::ensure!(!roots.is_empty(), "no {description}s found");
    Ok(roots)
}

fn load_certificates(
    path: &path::Path,
    description: &str,
) -> anyhow::Result<Vec<CertificateDer<'static>>> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to open {description}: {}", path.display()))?;
    let mut reader = io::BufReader::new(file);
    let certificates = rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("failed to read {description}: {}", path.display()))?;
    anyhow::ensure!(
        !certificates.is_empty(),
        "no {description}s found in {}",
        path.display()
    );
    Ok(certificates)
}

fn load_private_key(
    path: &path::Path,
    description: &str,
) -> anyhow::Result<PrivateKeyDer<'static>> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to open {description}: {}", path.display()))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    rustls_pemfile::private_key(&mut Cursor::new(bytes))?
        .with_context(|| format!("missing {description}: {}", path.display()))
}

fn load_identity(
    chain: &path::Path,
    key: &path::Path,
    description: &str,
) -> anyhow::Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    Ok((
        load_certificates(chain, &format!("{description} certificate"))?,
        load_private_key(key, &format!("{description} private key"))?,
    ))
}

#[derive(Default, Debug)]
struct ServeCerts {
    list: Vec<Arc<CertifiedKey>>,
}

impl ServeCerts {
    // Load a certificate and cooresponding key from a file
    pub fn load(&mut self, chain: &path::Path, key: &path::Path) -> anyhow::Result<()> {
        let (chain, key) = load_identity(chain, key, "TLS server identity")?;
        let key = rustls::crypto::ring::sign::any_supported_type(&key)?;

        let certified = Arc::new(CertifiedKey::new(chain, key));
        self.list.push(certified);

        Ok(())
    }

    // Return the SHA256 fingerprint of our certificates.
    pub fn fingerprints(&self) -> Vec<String> {
        self.list
            .iter()
            .map(|ck| {
                let fingerprint = digest(&SHA256, ck.cert[0].as_ref());
                let fingerprint = hex::encode(fingerprint.as_ref());
                fingerprint
            })
            .collect()
    }
}

/// Authenticated identity metadata retained for an accepted TLS peer.
///
/// Raw certificate bytes and subject names are deliberately omitted. The
/// complete SHA-256 leaf fingerprint is available to admission policies, while
/// `Debug` emits only a short prefix.
#[derive(Clone, Eq, PartialEq)]
pub struct CertificateIdentity {
    leaf_sha256: [u8; 32],
    chain_len: u8,
    total_der_bytes: u32,
}

impl CertificateIdentity {
    pub const MAX_CHAIN_LEN: usize = 8;
    pub const MAX_CERTIFICATE_BYTES: usize = 32 * 1024;
    pub const MAX_CHAIN_BYTES: usize = 64 * 1024;

    pub fn leaf_sha256(&self) -> &[u8; 32] {
        &self.leaf_sha256
    }

    pub fn leaf_sha256_hex(&self) -> String {
        hex::encode(self.leaf_sha256)
    }

    pub fn chain_len(&self) -> u8 {
        self.chain_len
    }

    pub fn total_der_bytes(&self) -> u32 {
        self.total_der_bytes
    }

    pub(crate) fn from_verified_chain(
        chain: &[CertificateDer<'_>],
    ) -> anyhow::Result<Option<Self>> {
        if chain.is_empty() {
            return Ok(None);
        }
        anyhow::ensure!(
            chain.len() <= Self::MAX_CHAIN_LEN,
            "peer certificate chain exceeds configured metadata bound"
        );
        anyhow::ensure!(
            chain
                .iter()
                .all(|certificate| certificate.as_ref().len() <= Self::MAX_CERTIFICATE_BYTES),
            "peer certificate exceeds configured metadata bound"
        );
        let total_der_bytes = chain
            .iter()
            .try_fold(0usize, |total, certificate| {
                total.checked_add(certificate.as_ref().len())
            })
            .context("peer certificate chain size overflow")?;
        anyhow::ensure!(
            total_der_bytes <= Self::MAX_CHAIN_BYTES,
            "peer certificate chain exceeds configured metadata bound"
        );

        let fingerprint = digest(&SHA256, chain[0].as_ref());
        let mut leaf_sha256 = [0u8; 32];
        leaf_sha256.copy_from_slice(fingerprint.as_ref());
        Ok(Some(Self {
            leaf_sha256,
            chain_len: chain.len() as u8,
            total_der_bytes: total_der_bytes as u32,
        }))
    }
}

impl std::fmt::Debug for CertificateIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let prefix = hex::encode(&self.leaf_sha256[..6]);
        formatter
            .debug_struct("CertificateIdentity")
            .field("leaf_sha256", &format_args!("{prefix}…"))
            .field("chain_len", &self.chain_len)
            .field("total_der_bytes", &self.total_der_bytes)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PeerIdentity {
    Anonymous,
    /// A chain was presented while certificate verification was explicitly
    /// disabled. It is diagnostic metadata only and is never authenticated.
    UnverifiedCertificate(CertificateIdentity),
    Certificate(CertificateIdentity),
}

impl PeerIdentity {
    pub fn certificate(&self) -> Option<&CertificateIdentity> {
        match self {
            Self::Anonymous | Self::UnverifiedCertificate(_) => None,
            Self::Certificate(identity) => Some(identity),
        }
    }

    pub fn presented_certificate(&self) -> Option<&CertificateIdentity> {
        match self {
            Self::Anonymous => None,
            Self::UnverifiedCertificate(identity) | Self::Certificate(identity) => Some(identity),
        }
    }

    pub fn is_authenticated(&self) -> bool {
        matches!(self, Self::Certificate(_))
    }
}

impl ResolvesServerCert for ServeCerts {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        if let Some(name) = client_hello.server_name() {
            if let Ok(dns_name) = webpki::DnsNameRef::try_from_ascii_str(name) {
                for ck in &self.list {
                    // TODO I gave up on caching the parsed result because of lifetime hell.
                    // If this shows up on benchmarks, somebody should fix it.
                    let leaf = ck.end_entity_cert().expect("missing certificate");
                    let parsed = webpki::EndEntityCert::try_from(leaf.as_ref())
                        .expect("failed to parse certificate");

                    if parsed.verify_is_valid_for_dns_name(dns_name).is_ok() {
                        return Some(ck.clone());
                    }
                }
            }
        }

        // Default to the last certificate if we couldn't find one.
        self.list.last().cloned()
    }
}

#[derive(Debug)]
pub struct NoCertificateVerification(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_client_auth_never_falls_back_to_platform_roots() {
        let error = Args {
            client_auth: ClientAuthMode::Required,
            ..Default::default()
        }
        .load()
        .err()
        .expect("required client auth without an explicit CA must fail");
        assert!(error.to_string().contains("--tls-client-ca"));
    }

    #[test]
    fn peer_certificate_metadata_is_bounded_and_redacted() {
        let leaf = CertificateDer::from(vec![0x42; 128]);
        let identity = CertificateIdentity::from_verified_chain(&[leaf])
            .unwrap()
            .unwrap();
        assert_eq!(identity.chain_len(), 1);
        assert_eq!(identity.total_der_bytes(), 128);
        let full_fingerprint = identity.leaf_sha256_hex();
        let debug = format!("{identity:?}");
        assert!(debug.contains('…'));
        assert!(!debug.contains(&full_fingerprint));

        let too_many = vec![CertificateDer::from(vec![1]); CertificateIdentity::MAX_CHAIN_LEN + 1];
        assert!(CertificateIdentity::from_verified_chain(&too_many).is_err());
        let oversized =
            CertificateDer::from(vec![0; CertificateIdentity::MAX_CERTIFICATE_BYTES + 1]);
        assert!(CertificateIdentity::from_verified_chain(&[oversized]).is_err());
    }
}
