//! Shared test PKI helpers — build a fully synthetic root + leaf
//! chain at test time with rcgen, and stamp SHAKEN-specific X.509
//! extensions (TNAuthList, JWT Claim Constraints) into the leaf via
//! `CustomExtension::from_oid_content`.
//!
//! These helpers carry no production dependencies; they live under
//! `tests/common/` and are included by test files via `mod common;`.

#![allow(dead_code)]

use async_trait::async_trait;
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, CustomExtension, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose, PKCS_ECDSA_P256_SHA256,
    PKCS_ECDSA_P384_SHA384,
};
use rvoip_stir_shaken::{CertResolver, VerifierError};
use std::sync::Arc;
use time::OffsetDateTime;
use url::Url;

/// One TNAuthList entry (RFC 8226 §9).
#[derive(Clone, Debug)]
pub enum TnAuthEntry {
    /// SPC entry — ambient authority for any TN.
    Spc(String),
    /// Single telephone number authorisation.
    Tn(String),
    /// `(start, count)` range authorisation.
    Range(String, u64),
}

/// Spec for the JWT Claim Constraints extension we want stamped in
/// the leaf. None of the entries are required by the cert profile;
/// when this list is empty, the extension is omitted entirely.
#[derive(Clone, Debug, Default)]
pub struct JccSpec {
    /// `(claim name, permitted UTF-8 string values)`.
    pub permitted: Vec<(String, Vec<String>)>,
}

/// Knobs the tests turn to build different leaf shapes.
#[derive(Clone, Debug)]
pub struct LeafSpec {
    /// Stamp a TNAuthList extension with these entries. Empty vec
    /// means omit the extension entirely (so the verifier reports
    /// "TNAuthList missing").
    pub tnauth_entries: Vec<TnAuthEntry>,
    /// Optional JWT Claim Constraints extension.
    pub jcc: Option<JccSpec>,
    /// Override the leaf's `not_after` date. `None` keeps rcgen's
    /// default (~1 year from now).
    pub not_after: Option<OffsetDateTime>,
    /// When `true`, the root key is P-384, so the chain signature
    /// over the leaf is ECDSA_P384_SHA384. The leaf key stays P-256
    /// (the JWS still signs ES256). Used to exercise webpki's
    /// ES256-only allow-list independently of the JWS path.
    pub p384_root: bool,
    /// Subject common name.
    pub common_name: String,
}

impl Default for LeafSpec {
    fn default() -> Self {
        Self {
            tnauth_entries: vec![TnAuthEntry::Spc("1234".into())],
            jcc: None,
            not_after: None,
            p384_root: false,
            common_name: "Test SHAKEN Leaf".into(),
        }
    }
}

/// A fully assembled test chain.
pub struct TestPki {
    pub root_kp: KeyPair,
    pub root_cert: Certificate,
    pub leaf_kp: KeyPair,
    pub leaf_cert: Certificate,
}

impl TestPki {
    /// Build a fresh root + leaf chain with the given leaf spec.
    pub fn build(spec: &LeafSpec) -> Self {
        // Root keypair drives the chain signature algorithm; leaf
        // key stays P-256 so the JWS can sign ES256 even when the
        // chain alg is non-P-256.
        let root_alg = if spec.p384_root {
            &PKCS_ECDSA_P384_SHA384
        } else {
            &PKCS_ECDSA_P256_SHA256
        };

        // --- Root (self-signed CA) ---
        let root_kp = KeyPair::generate_for(root_alg).expect("root keypair");
        let mut root_params =
            CertificateParams::new(vec!["Test STI-CA Root".into()]).expect("root params");
        root_params.distinguished_name = DistinguishedName::new();
        root_params
            .distinguished_name
            .push(DnType::CommonName, "Test STI-CA Root");
        root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        root_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let root_cert = root_params.self_signed(&root_kp).expect("self-sign root");
        let root_issuer = Issuer::from_params(&root_params, &root_kp);

        // --- Leaf (always P-256, regardless of root alg) ---
        let leaf_kp = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("leaf keypair");

        let mut leaf_params =
            CertificateParams::new(vec!["leaf.test".into()]).expect("leaf params");
        leaf_params.distinguished_name = DistinguishedName::new();
        leaf_params
            .distinguished_name
            .push(DnType::CommonName, &spec.common_name);
        leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        if let Some(not_after) = spec.not_after {
            leaf_params.not_after = not_after;
        }

        if !spec.tnauth_entries.is_empty() {
            leaf_params
                .custom_extensions
                .push(build_tnauth_extension(&spec.tnauth_entries));
        }
        if let Some(jcc) = &spec.jcc {
            if !jcc.permitted.is_empty() {
                leaf_params.custom_extensions.push(build_jcc_extension(jcc));
            }
        }

        let leaf_cert = leaf_params
            .signed_by(&leaf_kp, &root_issuer)
            .expect("sign leaf");

        Self {
            root_kp,
            root_cert,
            leaf_kp,
            leaf_cert,
        }
    }

    /// DER-encoded leaf cert (for the resolver to return).
    pub fn leaf_der(&self) -> Vec<u8> {
        self.leaf_cert.der().to_vec()
    }

    /// DER-encoded root cert (for the TrustStore).
    pub fn root_der(&self) -> Vec<u8> {
        self.root_cert.der().to_vec()
    }

    /// PEM-encoded leaf private key (for ShakenSigner::from_pem).
    pub fn leaf_key_pem(&self) -> String {
        self.leaf_kp.serialize_pem()
    }
}

/// Build a TNAuthList X.509 extension carrying the given entries
/// (RFC 8226 §9).
fn build_tnauth_extension(entries: &[TnAuthEntry]) -> CustomExtension {
    let mut body = Vec::new();
    for entry in entries {
        match entry {
            TnAuthEntry::Spc(spc) => push_tlv(&mut body, 0x80, spc.as_bytes()),
            TnAuthEntry::Tn(tn) => push_tlv(&mut body, 0x82, tn.as_bytes()),
            TnAuthEntry::Range(start, count) => {
                let mut inner = Vec::new();
                push_tlv(&mut inner, 0x16, start.as_bytes()); // IA5String start
                let count_bytes = encode_unsigned_integer(*count);
                push_tlv(&mut inner, 0x02, &count_bytes); // INTEGER count
                push_tlv(&mut body, 0xA1, &inner); // [1] SEQUENCE
            }
        }
    }
    let mut der = Vec::new();
    push_tlv(&mut der, 0x30, &body); // outer SEQUENCE

    // OID 1.3.6.1.5.5.7.1.26 — id-pe-TNAuthList
    let oid = vec![1, 3, 6, 1, 5, 5, 7, 1, 26];
    CustomExtension::from_oid_content(&oid, der)
}

/// Build a JWT Claim Constraints X.509 extension with permittedValues
/// only (mustInclude is not used by SHAKEN base profile).
fn build_jcc_extension(spec: &JccSpec) -> CustomExtension {
    let mut permitted_body = Vec::new();
    for (claim, values) in &spec.permitted {
        let mut entry = Vec::new();
        push_tlv(&mut entry, 0x16, claim.as_bytes()); // claim IA5String
        let mut values_body = Vec::new();
        for v in values {
            push_tlv(&mut values_body, 0x0C, v.as_bytes()); // UTF8String
        }
        push_tlv(&mut entry, 0x30, &values_body); // values SEQUENCE OF
        push_tlv(&mut permitted_body, 0x30, &entry); // JWTClaimValuesList SEQUENCE
    }
    let mut outer_body = Vec::new();
    push_tlv(&mut outer_body, 0xA1, &permitted_body); // [1] SEQUENCE OF
    let mut der = Vec::new();
    push_tlv(&mut der, 0x30, &outer_body); // outer SEQUENCE

    // OID 1.3.6.1.5.5.7.1.27 — id-pe-JWTClaimConstraints
    let oid = vec![1, 3, 6, 1, 5, 5, 7, 1, 27];
    CustomExtension::from_oid_content(&oid, der)
}

fn push_tlv(out: &mut Vec<u8>, tag: u8, body: &[u8]) {
    out.push(tag);
    let len = body.len();
    if len < 0x80 {
        out.push(len as u8);
    } else if len < 0x100 {
        out.push(0x81);
        out.push(len as u8);
    } else if len < 0x10000 {
        out.push(0x82);
        out.push((len >> 8) as u8);
        out.push((len & 0xFF) as u8);
    } else {
        panic!("test extension body > 64 KiB — unsupported");
    }
    out.extend_from_slice(body);
}

fn encode_unsigned_integer(n: u64) -> Vec<u8> {
    if n == 0 {
        return vec![0x00];
    }
    let mut bytes = Vec::new();
    let mut shift = 64;
    while shift > 0 {
        shift -= 8;
        let b = ((n >> shift) & 0xFF) as u8;
        if !bytes.is_empty() || b != 0 {
            bytes.push(b);
        }
    }
    // If top bit set, prepend a leading zero to keep the value
    // unsigned (DER convention).
    if bytes[0] & 0x80 != 0 {
        let mut zero_prefixed = vec![0x00];
        zero_prefixed.extend_from_slice(&bytes);
        zero_prefixed
    } else {
        bytes
    }
}

/// Resolver stub that returns the same DER bytes for every fetch.
pub struct StaticDerResolver {
    pub der: Vec<u8>,
}

#[async_trait]
impl CertResolver for StaticDerResolver {
    async fn fetch(&self, _url: &Url) -> Result<Vec<u8>, VerifierError> {
        Ok(self.der.clone())
    }
}

/// Wrap a DER cert blob in a resolver `Arc`.
pub fn resolver_for(der: Vec<u8>) -> Arc<dyn CertResolver> {
    Arc::new(StaticDerResolver { der })
}
