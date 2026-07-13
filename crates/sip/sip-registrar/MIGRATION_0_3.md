# SIP registrar credential-store migration

`UserStore` no longer retains recoverable plaintext passwords. Provisioning
APIs still accept a password, immediately derive the supported Digest HA1
verifiers, and zero the owned password allocation.

Code that previously called `get_password` must migrate to
`get_digest_secret(username, realm, algorithm)`. The deprecated
`get_password` method now returns an explicit `PlaintextCredentialUnavailable`
error for an existing user instead of silently returning `None`.

Code that previously called `get_credentials` must migrate to
`get_user_metadata`. The deprecated alias now returns
`UserCredentialMetadata`, which deliberately has no password field.

These source-visible changes are intentional security boundaries and require a
semver-breaking registrar release. They avoid the misleading compatibility
behavior where an existing user appeared to have no credentials.
