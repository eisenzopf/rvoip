//! Postgres-backed vCon persistence.

use async_trait::async_trait;
use rvoip_vcon::{content_hash, Vcon, VconStore, VconStoreError};
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
        vcon.validate()
            .map_err(|e| VconStoreError::Backend(format!("validate vcon: {e}")))?;
        let uuid = vcon.uuid;
        let encoded = serde_json::to_vec(&vcon)
            .map_err(|e| VconStoreError::Backend(format!("serialize vcon: {e}")))?;
        let json = serde_json::from_slice::<serde_json::Value>(&encoded)
            .map_err(|e| VconStoreError::Backend(format!("serialize vcon: {e}")))?;
        let hash = content_hash(&encoded);
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
        vcon.validate()
            .map_err(|e| VconStoreError::Backend(format!("validate vcon: {e}")))?;
        let uuid = vcon.uuid;
        let encoded = serde_json::to_vec(&vcon)
            .map_err(|e| VconStoreError::Backend(format!("serialize vcon: {e}")))?;
        let json = serde_json::from_slice::<serde_json::Value>(&encoded)
            .map_err(|e| VconStoreError::Backend(format!("serialize vcon: {e}")))?;
        let hash = content_hash(&encoded);
        let handle_url = format!("postgres:vcon/{uuid}");
        sqlx::query(
            "INSERT INTO rvoip_vcons (uuid, handle_url, vcon, content_hash)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (uuid) DO UPDATE SET
                handle_url = EXCLUDED.handle_url,
                vcon = EXCLUDED.vcon,
                vcon_bytes = NULL,
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

    async fn content_hash(&self, uuid: &Uuid) -> Result<String, VconStoreError> {
        PostgresVconStore::content_hash(self, uuid).await
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

fn to_store_error(err: sqlx::Error) -> VconStoreError {
    VconStoreError::Backend(err.to_string())
}

#[cfg(feature = "core-store")]
mod core_bridge {
    use super::*;
    use bytes::Bytes;
    use rvoip_core::error::{Result as CoreResult, RvoipError};
    use rvoip_core::ids::{ConversationId, SessionId, TenantId};
    use rvoip_core::store::{VconHandle, VconStore as CoreVconStore};

    #[async_trait]
    impl CoreVconStore for PostgresVconStore {
        async fn put(
            &self,
            tenant_id: &TenantId,
            conversation_id: &ConversationId,
            session_id: &SessionId,
            vcon_bytes: Bytes,
        ) -> CoreResult<VconHandle> {
            let vcon: Vcon = serde_json::from_slice(&vcon_bytes)
                .map_err(|error| RvoipError::Adapter(format!("invalid vcon JSON: {error}")))?;
            vcon.validate()
                .map_err(|error| RvoipError::Adapter(format!("invalid vcon: {error}")))?;
            let uuid = vcon.uuid;
            let json: serde_json::Value = serde_json::from_slice(&vcon_bytes)
                .map_err(|error| RvoipError::Adapter(format!("invalid vcon JSON: {error}")))?;
            let content_hash = content_hash(&vcon_bytes);
            let url = format!("postgres:vcon/{session_id}/{uuid}");
            sqlx::query(
                "INSERT INTO rvoip_vcons
                    (uuid, handle_url, tenant_id, conversation_id, session_id, vcon, vcon_bytes, content_hash)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(uuid)
            .bind(&url)
            .bind(tenant_id.to_string())
            .bind(conversation_id.to_string())
            .bind(session_id.to_string())
            .bind(json)
            .bind(vcon_bytes.as_ref())
            .bind(&content_hash)
            .execute(&self.pool)
            .await
            .map_err(to_core_error)?;
            Ok(VconHandle { url, content_hash })
        }

        async fn get(&self, handle: &VconHandle) -> CoreResult<Option<Bytes>> {
            let row = sqlx::query(
                "SELECT COALESCE(vcon_bytes, vcon_jws) AS vcon_bytes
                 FROM rvoip_vcons WHERE handle_url = $1",
            )
            .bind(&handle.url)
            .fetch_optional(&self.pool)
            .await
            .map_err(to_core_error)?;
            let Some(row) = row else {
                return Ok(None);
            };
            let bytes: Option<Vec<u8>> = row.try_get("vcon_bytes").map_err(to_core_error)?;
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

        async fn list_for_conversation(
            &self,
            conversation_id: &ConversationId,
        ) -> CoreResult<Vec<VconHandle>> {
            let rows = sqlx::query(
                "SELECT handle_url, content_hash
                 FROM rvoip_vcons
                 WHERE conversation_id = $1 AND handle_url IS NOT NULL
                 ORDER BY created_at ASC, uuid ASC",
            )
            .bind(conversation_id.to_string())
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
    #[cfg(feature = "live-tests")]
    use rvoip_vcon::{MemoryVconStore, Party, VconBuilder};

    #[cfg(feature = "live-tests")]
    fn database_url() -> String {
        std::env::var("DATABASE_URL")
            .ok()
            .filter(|value| !value.is_empty())
            .expect(
                "DATABASE_URL is required: PostgreSQL integration tests must run against an ephemeral live database",
            )
    }

    #[cfg(feature = "live-tests")]
    fn sample_vcon() -> Vcon {
        VconBuilder::new()
            .with_party(Party {
                name: Some("Alice".into()),
                ..Party::default()
            })
            .build()
    }

    #[test]
    fn migration_defines_expected_table() {
        assert!(MIGRATION_SQL.contains("CREATE TABLE IF NOT EXISTS rvoip_vcons"));
        assert!(MIGRATION_SQL.contains("uuid UUID PRIMARY KEY"));
        assert!(MIGRATION_SQL.contains("conversation_id TEXT"));
        assert!(MIGRATION_SQL.contains("vcon_bytes BYTEA"));
        assert!(MIGRATION_SQL.contains("content_hash TEXT NOT NULL"));
    }

    #[cfg(feature = "live-tests")]
    #[tokio::test]
    #[ignore = "requires DATABASE_URL pointing at an ephemeral PostgreSQL database"]
    async fn live_put_get_delete_list_and_hash() {
        let url = database_url();
        let store = PostgresVconStore::connect(&url).await.expect("connect");
        store.migrate().await.expect("migrate");

        let vcon = sample_vcon();
        let uuid = vcon.uuid;
        let expected_hash =
            content_hash(serde_json::to_vec(&vcon).expect("serialize expected vcon"));
        let memory = MemoryVconStore::new();
        memory.put(vcon.clone()).await.expect("memory put");
        assert_eq!(
            memory.content_hash(&uuid).await.expect("memory hash"),
            expected_hash
        );
        assert_eq!(store.put(vcon.clone()).await.expect("put"), uuid);
        let fetched = store.get(&uuid).await.expect("get");
        assert_eq!(fetched.uuid, uuid);
        assert_eq!(
            store.content_hash(&uuid).await.expect("hash"),
            expected_hash
        );
        assert_eq!(
            store.content_hash(&uuid).await.expect("postgres hash"),
            memory.content_hash(&uuid).await.expect("memory hash"),
            "typed memory and PostgreSQL stores must hash identical vCons identically"
        );

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

        #[cfg(feature = "core-store")]
        {
            use bytes::Bytes;
            use rvoip_core::ids::{ConversationId, SessionId, TenantId};
            use rvoip_core::store::VconStore as CoreVconStore;

            let core_vcon = sample_vcon();
            let body = Bytes::from(serde_json::to_vec(&core_vcon).expect("serialize core vcon"));
            let conversation_id = ConversationId::new();
            let session_id = SessionId::new();
            let handle = CoreVconStore::put(
                &store,
                &TenantId::new(),
                &conversation_id,
                &session_id,
                body.clone(),
            )
            .await
            .expect("core bridge put");
            assert_eq!(handle.content_hash, content_hash(&body));
            assert_eq!(
                CoreVconStore::get(&store, &handle)
                    .await
                    .expect("core bridge get"),
                Some(body)
            );
            assert_eq!(
                rvoip_vcon::VconStore::get(&store, &core_vcon.uuid)
                    .await
                    .expect("typed get of core-emitted vCon"),
                core_vcon
            );
            assert_eq!(
                CoreVconStore::list_for_session(&store, &session_id)
                    .await
                    .expect("list session")
                    .len(),
                1
            );
            assert_eq!(
                CoreVconStore::list_for_conversation(&store, &conversation_id)
                    .await
                    .expect("list conversation")
                    .len(),
                1
            );
        }
    }
}
