# rvoip-ldap

LDAP-backed `PasswordVerifier` implementation for legacy Basic-over-TLS
deployments.

This crate verifies a username/password by searching for one LDAP user entry
and then attempting an LDAP simple bind as that user. It is intended for
compatibility with OpenLDAP, 389 Directory Server, FreeIPA, Active
Directory-compatible LDAP services, and custom LDAP directories. SIP protocol
crates continue to depend only on `rvoip-auth-core::PasswordVerifier`.

Use the built-in presets instead of hand-writing common filters:

```rust,no_run
use rvoip_ldap::LdapPasswordVerifierConfig;

let openldap = LdapPasswordVerifierConfig::openldap(
    "ldap://127.0.0.1:1389",
    "ou=users,dc=rvoip,dc=local",
);
let directory_389 = LdapPasswordVerifierConfig::directory_389(
    "ldap://127.0.0.1:3389",
    "ou=people,dc=rvoip,dc=local",
);
let freeipa = LdapPasswordVerifierConfig::freeipa(
    "ldap://127.0.0.1:7389",
    "cn=users,cn=accounts,dc=rvoip,dc=local",
);
let ad = LdapPasswordVerifierConfig::active_directory(
    "ldaps://ad.example.com",
    "CN=Users,DC=example,DC=com",
);
```

Live tests skip unless LDAP environment variables are set. With the local
fixture:

```sh
cd ~/Developer/openldap
docker compose up -d

cd ~/Developer/rvoip
RVOIP_LDAP_URL=ldap://127.0.0.1:1389 \
RVOIP_LDAP_BIND_DN='cn=admin,dc=rvoip,dc=local' \
RVOIP_LDAP_BIND_PASSWORD=adminpassword \
RVOIP_LDAP_USER_BASE_DN='ou=users,dc=rvoip,dc=local' \
cargo test -p rvoip-ldap
```

Additional live-skipping fixture variables:

```sh
RVOIP_389DS_LDAP_URL=ldap://127.0.0.1:3389 \
RVOIP_389DS_USER_BASE_DN='ou=people,dc=rvoip,dc=local' \
cargo test -p rvoip-ldap

RVOIP_FREEIPA_LDAP_URL=ldap://127.0.0.1:7389 \
RVOIP_FREEIPA_USER_BASE_DN='cn=users,cn=accounts,dc=rvoip,dc=local' \
cargo test -p rvoip-ldap

RVOIP_AD_LDAP_URL=ldaps://ad.example.com \
RVOIP_AD_USER_BASE_DN='CN=Users,DC=example,DC=com' \
cargo test -p rvoip-ldap
```

For new CPaaS or enterprise applications, prefer OIDC/SAML for login and SCIM
for lifecycle provisioning. Use LDAP as a directory/password-verification
compatibility layer when an enterprise directory remains the source of truth.
