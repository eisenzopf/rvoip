---
section: Breaking changes
breaking: true
---
**`rvoip-users-core` and `rvoip-vcon-postgres` move to sqlx 0.9.**
`SqliteUserStore::pool`/`from_pool`, `PostgresUserStore::pool`,
`AuthenticationService::set_pool`, `PostgresVconStore::new`/`pool` and
`users_core::Error::Database` now carry sqlx 0.9 types. Migration: move
your own sqlx dependency to 0.9 when you pass pools in or match on the
database error.
