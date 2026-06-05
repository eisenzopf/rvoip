use sqlx_core::{query::query, raw_sql::raw_sql, row::Row};
use sqlx_sqlite::SqlitePool;
use tempfile::TempDir;
use users_core::SqliteUserStore;

fn db_url(temp_dir: &TempDir) -> String {
    let db_path = temp_dir.path().join("users.db");
    format!("sqlite://{}?mode=rwc", db_path.display())
}

fn expected_migrations() -> Vec<String> {
    [
        "001_initial_schema",
        "002_auth_security_tables",
        "003_api_key_active_state",
        "004_enterprise_identity_tables",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

async fn table_exists(pool: &SqlitePool, table: &str) -> bool {
    query("SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?")
        .bind(table)
        .fetch_optional(pool)
        .await
        .unwrap()
        .is_some()
}

async fn column_exists(pool: &SqlitePool, table: &str, column: &str) -> bool {
    let pragma = format!("PRAGMA table_info({table})");
    query(&pragma)
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .any(|row| row.get::<String, _>("name") == column)
}

async fn applied_migrations(pool: &SqlitePool) -> Vec<String> {
    query("SELECT id FROM schema_migrations ORDER BY id")
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get::<String, _>("id"))
        .collect()
}

async fn assert_migrated_schema(pool: &SqlitePool) {
    assert!(table_exists(pool, "schema_migrations").await);
    assert!(table_exists(pool, "revoked_access_tokens").await);
    assert!(table_exists(pool, "sip_digest_credentials").await);
    assert!(table_exists(pool, "external_identities").await);
    assert!(table_exists(pool, "passkey_credentials").await);
    assert!(column_exists(pool, "api_keys", "active").await);
}

#[tokio::test]
async fn fresh_database_runs_all_migrations() {
    let temp_dir = TempDir::new().unwrap();
    let store = SqliteUserStore::new(&db_url(&temp_dir)).await.unwrap();

    assert_migrated_schema(store.pool()).await;
    assert_eq!(
        applied_migrations(store.pool()).await,
        expected_migrations()
    );
}

#[tokio::test]
async fn old_shape_database_receives_auth_security_tables() {
    let temp_dir = TempDir::new().unwrap();
    let url = db_url(&temp_dir);
    let pool = SqlitePool::connect(&url).await.unwrap();
    raw_sql(
        "CREATE TABLE users (
            id TEXT PRIMARY KEY,
            username TEXT NOT NULL UNIQUE,
            email TEXT,
            display_name TEXT,
            password_hash TEXT NOT NULL,
            roles TEXT NOT NULL DEFAULT '[]',
            active BOOLEAN NOT NULL DEFAULT TRUE,
            created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            last_login TIMESTAMP
        );
        CREATE TABLE api_keys (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            name TEXT NOT NULL,
            key_hash TEXT NOT NULL UNIQUE,
            permissions TEXT NOT NULL DEFAULT '[]',
            expires_at TIMESTAMP,
            last_used TIMESTAMP,
            created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE refresh_tokens (
            jti TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            expires_at TIMESTAMP NOT NULL,
            revoked_at TIMESTAMP,
            created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE sessions (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            ip_address TEXT,
            user_agent TEXT,
            last_activity TIMESTAMP NOT NULL,
            expires_at TIMESTAMP NOT NULL,
            created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
        );",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let store = SqliteUserStore::new(&url).await.unwrap();
    assert_migrated_schema(store.pool()).await;
    assert_eq!(
        applied_migrations(store.pool()).await,
        expected_migrations()
    );
}

#[tokio::test]
async fn migrations_are_idempotent_on_reopen() {
    let temp_dir = TempDir::new().unwrap();
    let url = db_url(&temp_dir);
    {
        let store = SqliteUserStore::new(&url).await.unwrap();
        assert_eq!(
            applied_migrations(store.pool()).await.len(),
            expected_migrations().len()
        );
    }
    let store = SqliteUserStore::new(&url).await.unwrap();
    assert_migrated_schema(store.pool()).await;
    assert_eq!(
        applied_migrations(store.pool()).await,
        expected_migrations()
    );
}
