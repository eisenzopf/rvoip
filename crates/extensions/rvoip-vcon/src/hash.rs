use base64::Engine as _;
use sha2::{Digest, Sha512};

/// Encode bytes with the unpadded base64url alphabet used by vCon.
pub fn encode_base64url(bytes: impl AsRef<[u8]>) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes.as_ref())
}

/// Produce the mandatory SHA-512 external-content hash token.
pub fn content_hash(bytes: impl AsRef<[u8]>) -> String {
    let digest = Sha512::digest(bytes.as_ref());
    format!("sha512-{}", encode_base64url(digest))
}
