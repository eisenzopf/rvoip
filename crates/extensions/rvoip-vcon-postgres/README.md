# rvoip-vcon-postgres

Postgres-backed reference implementation for `rvoip_vcon::VconStore`.

The crate is optional and not required for in-process demos or tests. It stores typed vCon JSON in Postgres and exposes the migration SQL as `MIGRATION_SQL`. With the `core-store` feature, the same backend also implements the byte-oriented `rvoip_core::store::VconStore` bridge used by finalized recording artifacts.

Live tests are skipped unless `DATABASE_URL` points at a writable Postgres database.
