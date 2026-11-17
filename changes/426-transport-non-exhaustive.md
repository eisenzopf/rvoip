---
section: Breaking changes
breaking: true
---
**`rvoip_core_traits::Transport` is `#[non_exhaustive]`**, so later
transports are no longer breaking changes. Migration: add a wildcard arm
to `match`es on `Transport` outside `rvoip-core-traits`.
