use std::{fs, path::PathBuf};

use rcgen::{CertificateParams, KeyPair};
use tempfile::TempDir;

pub(crate) struct ServerIdentity {
    _directory: TempDir,
    pub(crate) cert: PathBuf,
    pub(crate) key: PathBuf,
}

pub(crate) fn localhost_server_identity() -> anyhow::Result<ServerIdentity> {
    let directory = tempfile::tempdir()?;
    let params = CertificateParams::new(vec!["localhost".to_owned()])?;
    let key_pair = KeyPair::generate()?;
    let certificate = params.self_signed(&key_pair)?;
    let cert = directory.path().join("localhost-cert.pem");
    let key = directory.path().join("localhost-key.pem");
    fs::write(&cert, certificate.pem())?;
    fs::write(&key, key_pair.serialize_pem())?;

    Ok(ServerIdentity {
        _directory: directory,
        cert,
        key,
    })
}
