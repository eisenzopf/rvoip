# Pinned vCon fixtures

`vcon_json_schema.json` is copied without semantic changes from
`ietf-wg-vcon/draft-ietf-vcon-vcon-core` commit
`2342aba64bdb71d9e80ab6e274a3921e2b1c769e`.

Source:
<https://github.com/ietf-wg-vcon/draft-ietf-vcon-vcon-core/blob/2342aba/vcon_json_schema.json>

`alice_email_curated.vcon` is derived from the same commit's
`ab_email_prob_followup_alice.vcon`. The example's obsolete reserved
`group` member and empty, schema-invalid `redacted` object were removed;
the conversational data is otherwise unchanged. The working-group
repository declares its documents under the IETF Trust Legal Provisions.

`jws-rs256-{private,public}-key.txt` (PEM-encoded key material) and
`jws-rs256-cert.der.b64` are a self-signed RSA test identity generated solely
for offline JWS conformance tests. They are public test material and are not
trusted by any runtime path.
