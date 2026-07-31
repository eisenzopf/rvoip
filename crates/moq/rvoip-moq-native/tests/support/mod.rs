// SPDX-FileCopyrightText: 2026 Bridgefu contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{fs, path::PathBuf};

use moq_native_ietf::tls;
use rcgen::{CertificateParams, ExtendedKeyUsagePurpose, KeyPair, KeyUsagePurpose};
use time::OffsetDateTime;

pub fn localhost_server_tls(
    client_auth: tls::ClientAuthMode,
    client_ca: &[PathBuf],
) -> anyhow::Result<tls::Config> {
    let directory = tempfile::tempdir()?;
    let mut params = CertificateParams::new(vec!["localhost".to_owned()])?;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.not_before = OffsetDateTime::now_utc() - time::Duration::days(1);
    params.not_after = OffsetDateTime::now_utc() + time::Duration::days(1);

    let key = KeyPair::generate()?;
    let certificate = params.self_signed(&key)?;
    let cert_path = directory.path().join("localhost-cert.pem");
    let key_path = directory.path().join("localhost-key.pem");
    fs::write(&cert_path, certificate.pem())?;
    fs::write(&key_path, key.serialize_pem())?;

    tls::Args {
        cert: vec![cert_path],
        key: vec![key_path],
        client_auth,
        client_ca: client_ca.to_vec(),
        disable_verify: true,
        ..Default::default()
    }
    .load()
}
