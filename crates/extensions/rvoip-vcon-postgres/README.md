# rvoip-vcon-postgres

Postgres-backed reference implementation for `rvoip_vcon::VconStore`.

The crate is optional and not required for in-process demos or tests. It stores typed vCon JSON in Postgres and exposes the migration SQL as `MIGRATION_SQL`. With the `core-store` feature, the same backend also implements the byte-oriented `rvoip_core::store::VconStore` bridge used to persist vCons during session finalization.

Live database coverage is opt-in and fail-closed:

```sh
DATABASE_URL=postgres://... \
  cargo test -p rvoip-vcon-postgres --all-targets \
  --features core-store,live-tests --locked
```

Without `live-tests`, service-free workspace test runs omit the database test.
With `live-tests`, a missing or empty `DATABASE_URL` fails the test instead of
silently skipping it.
