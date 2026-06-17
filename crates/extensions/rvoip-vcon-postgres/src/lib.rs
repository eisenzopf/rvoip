//! Postgres-backed vCon persistence.

use async_trait::async_trait;
use rvoip_vcon::{Vcon, VconStore, VconStoreError};
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use uuid::Uuid;

pub const MIGRATION_SQL: &str = include_str!("../migrations/0001_vcon_store.sql");

#[derive(Clone)]
pub struct PostgresVconStore {
    pool: PgPool,
}

impl PostgresVconStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn connect(database_url: &str) -> Result<Self, VconStoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .map_err(to_store_error)?;
        Ok(Self::new(pool))
    }

    pub async fn migrate(&self) -> Result<(), VconStoreError> {
        for statement in MIGRATION_SQL.split(';').map(str::trim) {
            if statement.is_empty() {
                continue;
            }
            sqlx::query(statement)
                .execute(&self.pool)
                .await
                .map_err(to_store_error)?;
        }
        Ok(())
    }

    pub async fn content_hash(&self, uuid: &Uuid) -> Result<String, VconStoreError> {
        let row = sqlx::query("SELECT content_hash FROM rvoip_vcons WHERE uuid = $1")
            .bind(uuid)
            .fetch_optional(&self.pool)
            .await
            .map_err(to_store_error)?;
        row.map(|r| r.get::<String, _>("content_hash"))
            .ok_or(VconStoreError::NotFound(*uuid))
    }
}

#[async_trait]
impl VconStore for PostgresVconStore {
    async fn put(&self, vcon: Vcon) -> Result<Uuid, VconStoreError> {
        let uuid = vcon.uuid;
        let json = serde_json::to_value(&vcon)
            .map_err(|e| VconStoreError::Backend(format!("serialize vcon: {e}")))?;
        let hash = sha256_json(&json)?;
        let handle_url = format!("postgres:vcon/{uuid}");
        sqlx::query(
            "INSERT INTO rvoip_vcons (uuid, handle_url, vcon, content_hash)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(uuid)
        .bind(handle_url)
        .bind(json)
        .bind(hash)
        .execute(&self.pool)
        .await
        .map_err(to_store_error)?;
        Ok(uuid)
    }

    async fn put_overwrite(&self, vcon: Vcon) -> Result<Uuid, VconStoreError> {
        let uuid = vcon.uuid;
        let json = serde_json::to_value(&vcon)
            .map_err(|e| VconStoreError::Backend(format!("serialize vcon: {e}")))?;
        let hash = sha256_json(&json)?;
        let handle_url = format!("postgres:vcon/{uuid}");
        sqlx::query(
            "INSERT INTO rvoip_vcons (uuid, handle_url, vcon, content_hash)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (uuid) DO UPDATE SET
                handle_url = EXCLUDED.handle_url,
                vcon = EXCLUDED.vcon,
                vcon_jws = NULL,
                content_hash = EXCLUDED.content_hash,
                updated_at = now()",
        )
        .bind(uuid)
        .bind(handle_url)
        .bind(json)
        .bind(hash)
        .execute(&self.pool)
        .await
        .map_err(to_store_error)?;
        Ok(uuid)
    }

    async fn get(&self, uuid: &Uuid) -> Result<Vcon, VconStoreError> {
        let row = sqlx::query("SELECT vcon FROM rvoip_vcons WHERE uuid = $1")
            .bind(uuid)
            .fetch_optional(&self.pool)
            .await
            .map_err(to_store_error)?;
        let Some(row) = row else {
            return Err(VconStoreError::NotFound(*uuid));
        };
        let value: Option<serde_json::Value> = row
            .try_get("vcon")
            .map_err(|e| VconStoreError::Backend(e.to_string()))?;
        let value = value.ok_or(VconStoreError::NotFound(*uuid))?;
        serde_json::from_value(value)
            .map_err(|e| VconStoreError::Backend(format!("deserialize vcon: {e}")))
    }

    async fn delete(&self, uuid: &Uuid) -> Result<(), VconStoreError> {
        sqlx::query("DELETE FROM rvoip_vcons WHERE uuid = $1")
            .bind(uuid)
            .execute(&self.pool)
            .await
            .map_err(to_store_error)?;
        Ok(())
    }

    async fn len(&self) -> Option<usize> {
        let row = sqlx::query("SELECT COUNT(*) AS n FROM rvoip_vcons")
            .fetch_one(&self.pool)
            .await
            .ok()?;
        let n: i64 = row.try_get("n").ok()?;
        usize::try_from(n).ok()
    }
}

fn sha256_json(value: &serde_json::Value) -> Result<String, VconStoreError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|e| VconStoreError::Backend(format!("serialize vcon hash: {e}")))?;
    Ok(format!("sha256:{}", sha256_hex(&bytes)))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    hex_encode(digest)
}

fn hex_encode(bytes: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

fn to_store_error(err: sqlx::Error) -> VconStoreError {
    VconStoreError::Backend(err.to_string())
}

#[cfg(feature = "core-store")]
mod core_bridge {
    use super::*;
    use bytes::Bytes;
    use rvoip_core::error::{Result as CoreResult, RvoipError};
    use rvoip_core::ids::{SessionId, TenantId};
    use rvoip_core::store::{VconHandle, VconStore as CoreVconStore};

    #[async_trait]
    impl CoreVconStore for PostgresVconStore {
        async fn put(
            &self,
            tenant_id: &TenantId,
            session_id: &SessionId,
            vcon_jws: Bytes,
        ) -> CoreResult<VconHandle> {
            let uuid = Uuid::new_v4();
            let content_hash = format!("sha256:{}", sha256_hex(&vcon_jws));
            let url = format!("postgres:vcon/{session_id}/{uuid}");
            sqlx::query(
                "INSERT INTO rvoip_vcons
                    (uuid, handle_url, tenant_id, session_id, vcon_jws, content_hash)
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(uuid)
            .bind(&url)
            .bind(tenant_id.to_string())
            .bind(session_id.to_string())
            .bind(vcon_jws.as_ref())
            .bind(&content_hash)
            .execute(&self.pool)
            .await
            .map_err(to_core_error)?;
            Ok(VconHandle { url, content_hash })
        }

        async fn get(&self, handle: &VconHandle) -> CoreResult<Option<Bytes>> {
            let row = sqlx::query("SELECT vcon_jws FROM rvoip_vcons WHERE handle_url = $1")
                .bind(&handle.url)
                .fetch_optional(&self.pool)
                .await
                .map_err(to_core_error)?;
            let Some(row) = row else {
                return Ok(None);
            };
            let bytes: Option<Vec<u8>> = row.try_get("vcon_jws").map_err(to_core_error)?;
            Ok(bytes.map(Bytes::from))
        }

        async fn list_for_session(&self, session_id: &SessionId) -> CoreResult<Vec<VconHandle>> {
            let rows = sqlx::query(
                "SELECT handle_url, content_hash
                 FROM rvoip_vcons
                 WHERE session_id = $1 AND handle_url IS NOT NULL
                 ORDER BY created_at ASC, uuid ASC",
            )
            .bind(session_id.to_string())
            .fetch_all(&self.pool)
            .await
            .map_err(to_core_error)?;
            Ok(rows
                .into_iter()
                .map(|row| VconHandle {
                    url: row.get("handle_url"),
                    content_hash: row.get("content_hash"),
                })
                .collect())
        }
    }

    fn to_core_error(err: sqlx::Error) -> RvoipError {
        RvoipError::Adapter(format!("postgres vcon store: {err}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_vcon::{Party, VconBuilder};

    fn database_url() -> Option<String> {
        std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty())
    }

    fn sample_vcon() -> Vcon {
        VconBuilder::new()
            .with_party(Party {
                name: Some("Alice".into()),
                role: Some("caller".into()),
                ..Party::default()
            })
            .build()
    }

    #[test]
    fn migration_defines_expected_table() {
        assert!(MIGRATION_SQL.contains("CREATE TABLE IF NOT EXISTS rvoip_vcons"));
        assert!(MIGRATION_SQL.contains("uuid UUID PRIMARY KEY"));
        assert!(MIGRATION_SQL.contains("content_hash TEXT NOT NULL"));
    }

    #[tokio::test]
    async fn live_put_get_delete_list_and_hash() {
        let Some(url) = database_url() else {
            return;
        };
        let store = PostgresVconStore::connect(&url).await.expect("connect");
        store.migrate().await.expect("migrate");

        let vcon = sample_vcon();
        let uuid = vcon.uuid;
        assert_eq!(store.put(vcon.clone()).await.expect("put"), uuid);
        let fetched = store.get(&uuid).await.expect("get");
        assert_eq!(fetched.uuid, uuid);
        assert!(store
            .content_hash(&uuid)
            .await
            .expect("hash")
            .starts_with("sha256:"));

        let duplicate = store.put(vcon.clone()).await;
        assert!(duplicate.is_err(), "duplicate uuid should fail");

        let mut overwritten = vcon;
        overwritten.subject = Some("updated".into());
        store
            .put_overwrite(overwritten)
            .await
            .expect("put overwrite");
        assert_eq!(
            store.get(&uuid).await.expect("get overwritten").subject,
            Some("updated".into())
        );

        store.delete(&uuid).await.expect("delete");
        assert!(matches!(
            store.get(&uuid).await,
            Err(VconStoreError::NotFound(id)) if id == uuid
        ));
    }
}
