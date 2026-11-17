---
section: Breaking changes
breaking: true
---
**validator 0.20 → 0.21.** `rvoip_users_core::validation::ValidatedCreateUserRequest`
implements validator 0.21's `Validate`. Migration: depend on validator 0.21
if you call `.validate()` through your own validator import.
