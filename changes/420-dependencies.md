---
section: Changed
breaking: false
---
**jsonwebtoken** 10.4 → 12.0 in auth-core, users-core, vcon, stir-shaken,
webex-byova, moq and uctp (11 is skipped). 12.0 fixes the remaining
`sub`/`iss`/`aud` type confusion, rejects unknown `crit` header extensions and
refuses empty HMAC keys.
