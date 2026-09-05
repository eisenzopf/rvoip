# Sig9421 cross-language fixture

`sig9421-cross-language-v1.json` is the byte-for-byte Ed25519 fixture for
RVoIP's inline signed-envelope profile. It uses the RFC 8032 test-vector seed;
the private seed is test material only and MUST NOT be used outside tests.

Every implementation MUST:

1. remove the top-level `signature` member if it is present;
2. serialize the remaining value using RFC 8785/JCS to UTF-8;
3. assert that the bytes equal `canonical_utf8` exactly;
4. sign those bytes with Ed25519; and
5. encode the signature as unpadded base64url and assert exact equality with
   `signature_base64url_no_pad`.

Known implementation mappings:

| Language | RFC 8785 implementation | Ed25519 operation |
|---|---|---|
| Rust | `serde_jcs::to_vec` | `ring::signature::Ed25519KeyPair::sign` |
| JavaScript | `canonicalize` (RFC 8785 package) | `crypto.sign(null, bytes, key)` |
| Python | `rfc8785.dumps` | `Ed25519PrivateKey.sign` from `cryptography` |
| Go | `jsoncanonicalizer.Transform` | `ed25519.Sign` |

The Rust conformance test in `src/sig9421.rs` pins the exact canonical bytes,
public key, and signature. JavaScript, Python, and Go clients consume the same
JSON file; a mismatch is a protocol failure, not a value to normalize after
signing.
