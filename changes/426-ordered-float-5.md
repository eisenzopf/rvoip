---
section: Breaking changes
breaking: true
---
**ordered-float 4 → 5 in `rvoip-sip-core`.** `Contact::q`,
`Address::q`, `AcceptLanguage`'s `q`, and `Timestamp::new` / `value` /
`delay` / `set_value` / `set_delay` use ordered-float 5's `NotNan`.
Migration: depend on ordered-float 5 where you name `NotNan`.
