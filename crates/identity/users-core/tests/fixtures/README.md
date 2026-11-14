# users-core test fixtures

`rs256-test-private-key-{pkcs8,pkcs1}.txt` are the same 2048-bit RSA key in
PKCS#8 and PKCS#1 PEM form, generated solely for the offline RS256 token tests
in `tests/rs256_tokens.rs`. They are public test material and are not trusted
by any runtime path.
