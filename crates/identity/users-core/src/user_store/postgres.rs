//! PostgreSQL user/API-key store.

use crate::{CreateUserRequest, Error, Result, UpdateUserRequest, User, UserFilter};
use async_trait::async_trait;
use chrono::Utc;
use sqlx_core::{query::query, raw_sql::raw_sql, row::Row};
use sqlx_postgres::{PgPool, PgRow};

const POSTGRES_MIGRATIONS: &[(&str, &str)] = &[
    (
        "001_initial_schema",
        include_str!("../../migrations/postgres/001_initial_schema.sql"),
    ),
    (
        "002_auth_security_tables",
        include_str!("../../migrations/postgres/002_auth_security_tables.sql"),
    ),
    (
        "003_api_key_active_state",
        include_str!("../../migrations/postgres/003_api_key_active_state.sql"),
    ),
    (
        "004_enterprise_identity_tables",
        include_str!("../../migrations/postgres/004_enterprise_identity_tables.sql"),
    ),
];

/// PostgreSQL-backed user store.
#[derive(Clone)]
pub struct PostgresUserStore {
    pool: PgPool,
}

impl PostgresUserStore {
    /// Connect to PostgreSQL and run users-core migrations.
    pub async fn new(database_url: &str) -> Result<Self> {
        let pool = PgPool::connect(database_url)
            .await
            .map_err(Error::Database)?;
        run_postgres_migrations(&pool).await?;
        Ok(Self { pool })
    }

    /// Get the underlying pool.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    fn row_to_user(&self, row: PgRow) -> User {
        User {
            id: row.get("id"),
            username: row.get("username"),
            email: row.get("email"),
            display_name: row.get("display_name"),
            password_hash: row.get("password_hash"),
            roles: serde_json::from_str(row.get("roles")).unwrap_or_default(),
            active: row.get("active"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
            last_login: row.get("last_login"),
        }
    }
}

async fn run_postgres_migrations(pool: &PgPool) -> Result<()> {
    raw_sql(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            id TEXT PRIMARY KEY,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
        );",
    )
    .execute(pool)
    .await?;

    for (id, sql) in POSTGRES_MIGRATIONS {
        let applied = query("SELECT 1 FROM schema_migrations WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?
            .is_some();
        if applied {
            continue;
        }

        raw_sql(sql).execute(pool).await?;
        query("INSERT INTO schema_migrations (id) VALUES ($1) ON CONFLICT (id) DO NOTHING")
            .bind(id)
            .execute(pool)
            .await?;
    }

    Ok(())
}

#[async_trait]
impl crate::UserStore for PostgresUserStore {
    async fn create_user(&self, request: CreateUserRequest) -> Result<User> {
        if self
            .get_user_by_username(&request.username)
            .await?
            .is_some()
        {
            return Err(Error::UserAlreadyExists(request.username));
        }

        let user = User {
            id: User::new_id(),
            username: request.username,
            email: request.email,
            display_name: request.display_name,
            password_hash: request.password,
            roles: request.roles,
            active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_login: None,
        };
        let roles_json = serde_json::to_string(&user.roles).unwrap();

        query(
            "INSERT INTO users
                (id, username, email, display_name, password_hash, roles, active, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(&user.id)
        .bind(&user.username)
        .bind(&user.email)
        .bind(&user.display_name)
        .bind(&user.password_hash)
        .bind(&roles_json)
        .bind(user.active)
        .bind(user.created_at)
        .bind(user.updated_at)
        .execute(&self.pool)
        .await?;

        Ok(user)
    }

    async fn get_user(&self, id: &str) -> Result<Option<User>> {
        let row = query(
            "SELECT id, username, email, display_name, password_hash, roles, active, created_at, updated_at, last_login
             FROM users WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| self.row_to_user(row)))
    }

    async fn get_user_by_username(&self, username: &str) -> Result<Option<User>> {
        let row = query(
            "SELECT id, username, email, display_name, password_hash, roles, active, created_at, updated_at, last_login
             FROM users WHERE username = $1",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| self.row_to_user(row)))
    }

    async fn update_user(&self, id: &str, updates: UpdateUserRequest) -> Result<User> {
        let mut user = self
            .get_user(id)
            .await?
            .ok_or_else(|| Error::UserNotFound(id.to_string()))?;

        if let Some(email) = updates.email {
            user.email = Some(email);
        }
        if let Some(display_name) = updates.display_name {
            user.display_name = Some(display_name);
        }
        if let Some(roles) = updates.roles {
            user.roles = roles;
        }
        if let Some(active) = updates.active {
            user.active = active;
        }

        user.updated_at = Utc::now();
        let roles_json = serde_json::to_string(&user.roles).unwrap();
        query(
            "UPDATE users
             SET email = $1, display_name = $2, roles = $3, active = $4, updated_at = $5
             WHERE id = $6",
        )
        .bind(&user.email)
        .bind(&user.display_name)
        .bind(&roles_json)
        .bind(user.active)
        .bind(user.updated_at)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(user)
    }

    async fn delete_user(&self, id: &str) -> Result<()> {
        let result = query("DELETE FROM users WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(Error::UserNotFound(id.to_string()));
        }
        Ok(())
    }

    async fn list_users(&self, filter: UserFilter) -> Result<Vec<User>> {
        let mut query_str = String::from(
            "SELECT id, username, email, display_name, password_hash, roles, active, created_at, updated_at, last_login
             FROM users WHERE 1=1",
        );
        let mut params = Vec::new();

        if let Some(active) = filter.active {
            query_str.push_str(&format!(" AND active = ${}", params.len() + 1));
            params.push(PostgresParam::Bool(active));
        }

        if let Some(role) = filter.role {
            let safe_role = role.replace(['"', '%', '\''], "");
            query_str.push_str(&format!(" AND roles LIKE ${}", params.len() + 1));
            params.push(PostgresParam::Text(format!("%\"{}%", safe_role)));
        }

        if let Some(search) = filter.search {
            if search.len() > 100 {
                return Err(Error::Validation("Search term too long".to_string()));
            }
            let safe_search = search
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
                .replace('\'', "''");
            let pattern = format!("%{}%", safe_search);
            let username_param = params.len() + 1;
            let email_param = params.len() + 2;
            let display_param = params.len() + 3;
            query_str.push_str(&format!(
                " AND (username LIKE ${username_param} ESCAPE '\\' OR email LIKE ${email_param} ESCAPE '\\' OR display_name LIKE ${display_param} ESCAPE '\\')"
            ));
            params.push(PostgresParam::Text(pattern.clone()));
            params.push(PostgresParam::Text(pattern.clone()));
            params.push(PostgresParam::Text(pattern));
        }

        query_str.push_str(" ORDER BY created_at DESC");

        if let Some(limit) = filter.limit {
            if limit > 1000 {
                return Err(Error::Validation("Limit too large".to_string()));
            }
            query_str.push_str(&format!(" LIMIT ${}", params.len() + 1));
            params.push(PostgresParam::I64(limit as i64));
        }

        if let Some(offset) = filter.offset {
            if offset > 100000 {
                return Err(Error::Validation("Offset too large".to_string()));
            }
            query_str.push_str(&format!(" OFFSET ${}", params.len() + 1));
            params.push(PostgresParam::I64(offset as i64));
        }

        let mut query = query(&query_str);
        for param in params {
            query = match param {
                PostgresParam::Text(value) => query.bind(value),
                PostgresParam::Bool(value) => query.bind(value),
                PostgresParam::I64(value) => query.bind(value),
            };
        }

        let rows = query.fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(|row| self.row_to_user(row)).collect())
    }
}

enum PostgresParam {
    Text(String),
    Bool(bool),
    I64(i64),
}

#[async_trait]
impl crate::ApiKeyStore for PostgresUserStore {
    async fn create_api_key(
        &self,
        request: crate::api_keys::CreateApiKeyRequest,
    ) -> Result<(crate::ApiKey, String)> {
        use rand::Rng;
        use sha2::{Digest, Sha256};

        request.validate()?;

        let raw_key = format!(
            "rvoip_ak_live_{}",
            rand::thread_rng()
                .sample_iter(&rand::distributions::Alphanumeric)
                .take(32)
                .map(char::from)
                .collect::<String>()
        );
        let mut hasher = Sha256::new();
        hasher.update(raw_key.as_bytes());
        let key_hash = format!("{:x}", hasher.finalize());

        let api_key = crate::ApiKey {
            id: crate::User::new_id(),
            user_id: request.user_id,
            name: request.name,
            key_hash: key_hash.clone(),
            permissions: request.permissions,
            active: true,
            expires_at: request.expires_at,
            last_used: None,
            created_at: Utc::now(),
        };
        let permissions_json = serde_json::to_string(&api_key.permissions).unwrap();

        query(
            "INSERT INTO api_keys
                (id, user_id, name, key_hash, permissions, active, expires_at, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(&api_key.id)
        .bind(&api_key.user_id)
        .bind(&api_key.name)
        .bind(&api_key.key_hash)
        .bind(&permissions_json)
        .bind(api_key.active)
        .bind(api_key.expires_at)
        .bind(api_key.created_at)
        .execute(&self.pool)
        .await?;

        Ok((api_key, raw_key))
    }

    async fn validate_api_key(&self, key: &str) -> Result<Option<crate::ApiKey>> {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        hasher.update(key.as_bytes());
        let key_hash = format!("{:x}", hasher.finalize());

        let row = query(
            "SELECT id, user_id, name, key_hash, permissions, active, expires_at, last_used, created_at
             FROM api_keys WHERE key_hash = $1",
        )
        .bind(&key_hash)
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };
        let mut api_key = row_to_api_key(row);
        if !api_key.active {
            return Ok(None);
        }
        if let Some(expires_at) = api_key.expires_at {
            if expires_at < Utc::now() {
                return Err(Error::ApiKeyExpired);
            }
        }

        let now = Utc::now();
        query("UPDATE api_keys SET last_used = $1 WHERE id = $2")
            .bind(now)
            .bind(&api_key.id)
            .execute(&self.pool)
            .await?;
        api_key.last_used = Some(now);
        Ok(Some(api_key))
    }

    async fn set_api_key_active(&self, id: &str, active: bool) -> Result<()> {
        let result = query("UPDATE api_keys SET active = $1 WHERE id = $2")
            .bind(active)
            .bind(id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(Error::ApiKeyNotFound);
        }
        Ok(())
    }

    async fn revoke_api_key(&self, id: &str) -> Result<()> {
        let result = query("DELETE FROM api_keys WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(Error::ApiKeyNotFound);
        }
        Ok(())
    }

    async fn list_api_keys(&self, user_id: &str) -> Result<Vec<crate::ApiKey>> {
        let rows = query(
            "SELECT id, user_id, name, key_hash, permissions, active, expires_at, last_used, created_at
             FROM api_keys WHERE user_id = $1 ORDER BY created_at DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(row_to_api_key).collect())
    }
}

fn row_to_api_key(row: PgRow) -> crate::ApiKey {
    crate::ApiKey {
        id: row.get("id"),
        user_id: row.get("user_id"),
        name: row.get("name"),
        key_hash: row.get("key_hash"),
        permissions: serde_json::from_str(row.get("permissions")).unwrap_or_default(),
        active: row.get("active"),
        expires_at: row.get("expires_at"),
        last_used: row.get("last_used"),
        created_at: row.get("created_at"),
    }
}
