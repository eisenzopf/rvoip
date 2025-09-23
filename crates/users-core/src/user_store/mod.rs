//! User storage interface

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{SqlitePool, Row};
use crate::{Result, Error, User, CreateUserRequest, UpdateUserRequest, UserFilter};

/// User storage trait
#[async_trait]
pub trait UserStore: Send + Sync {
    async fn create_user(&self, request: CreateUserRequest) -> Result<User>;
    async fn get_user(&self, id: &str) -> Result<Option<User>>;
    async fn get_user_by_username(&self, username: &str) -> Result<Option<User>>;
    async fn update_user(&self, id: &str, updates: UpdateUserRequest) -> Result<User>;
    async fn delete_user(&self, id: &str) -> Result<()>;
    async fn list_users(&self, filter: UserFilter) -> Result<Vec<User>>;
}

/// SQLite-based user store
#[derive(Clone)]
pub struct SqliteUserStore {
    pool: SqlitePool,
}

impl SqliteUserStore {
    pub async fn new(database_url: &str) -> Result<Self> {
        // Create the database pool
        let pool = SqlitePool::connect(database_url)
            .await
            .map_err(|e| Error::Database(e))?;
        
        // Run migrations
        let migration_sql = include_str!("../../migrations/001_initial_schema.sql");
        sqlx::raw_sql(migration_sql)
            .execute(&pool)
            .await
            .map_err(|e| Error::Database(e))?;
        
        Ok(Self { pool })
    }
    
    /// Get the underlying pool
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

// Implement UserStore trait
#[async_trait]
impl UserStore for SqliteUserStore {
    async fn create_user(&self, request: CreateUserRequest) -> Result<User> {
        // Check if username already exists
        let existing = self.get_user_by_username(&request.username).await?;
        if existing.is_some() {
            return Err(Error::UserAlreadyExists(request.username));
        }
        
        let user = User {
            id: User::new_id(),
            username: request.username,
            email: request.email,
            display_name: request.display_name,
            password_hash: request.password,  // Note: This should be hashed by AuthenticationService
            roles: request.roles,
            active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_login: None,
        };
        
        let roles_json = serde_json::to_string(&user.roles).unwrap();
        
        sqlx::query(
            "INSERT INTO users (id, username, email, display_name, password_hash, roles, active, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(&user.id)
        .bind(&user.username)
        .bind(&user.email)
        .bind(&user.display_name)
        .bind(&user.password_hash)
        .bind(&roles_json)
        .bind(user.active)
        .bind(&user.created_at)
        .bind(&user.updated_at)
        .execute(&self.pool)
        .await?;
        
        Ok(user)
    }
    
    async fn get_user(&self, id: &str) -> Result<Option<User>> {
        let row = sqlx::query(
            "SELECT id, username, email, display_name, password_hash, roles, active, created_at, updated_at, last_login
             FROM users WHERE id = ?"
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        
        Ok(row.map(|row| self.row_to_user(row)))
    }
    
    async fn get_user_by_username(&self, username: &str) -> Result<Option<User>> {
        let row = sqlx::query(
            "SELECT id, username, email, display_name, password_hash, roles, active, created_at, updated_at, last_login
             FROM users WHERE username = ?"
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;
        
        Ok(row.map(|row| self.row_to_user(row)))
    }
    
    async fn update_user(&self, id: &str, updates: UpdateUserRequest) -> Result<User> {
        let mut user = self.get_user(id).await?
            .ok_or_else(|| Error::UserNotFound(id.to_string()))?;
        
        // Apply updates
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
        
        sqlx::query(
            "UPDATE users SET email = ?, display_name = ?, roles = ?, active = ?, updated_at = ?
             WHERE id = ?"
        )
        .bind(&user.email)
        .bind(&user.display_name)
        .bind(&roles_json)
        .bind(user.active)
        .bind(&user.updated_at)
        .bind(id)
        .execute(&self.pool)
        .await?;
        
        Ok(user)
    }
    
    async fn delete_user(&self, id: &str) -> Result<()> {
        let result = sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        
        if result.rows_affected() == 0 {
            return Err(Error::UserNotFound(id.to_string()));
        }
        
        Ok(())
    }
    
    async fn list_users(&self, filter: UserFilter) -> Result<Vec<User>> {
        let mut query = String::from(
            "SELECT id, username, email, display_name, password_hash, roles, active, created_at, updated_at, last_login
             FROM users WHERE 1=1"
        );
        
        // Build dynamic query based on filter
        let mut bindings = vec![];
        
        if let Some(active) = filter.active {
            query.push_str(" AND active = ?");
            bindings.push(if active { "1" } else { "0" }.to_string());
        }
        
        if let Some(role) = filter.role {
            query.push_str(" AND roles LIKE ?");
            bindings.push(format!("%\"{}%", role));
        }
        
        if let Some(search) = filter.search {
            query.push_str(" AND (username LIKE ? OR email LIKE ? OR display_name LIKE ?)");
            let search_pattern = format!("%{}%", search);
            bindings.push(search_pattern.clone());
            bindings.push(search_pattern.clone());
            bindings.push(search_pattern);
        }
        
        query.push_str(" ORDER BY created_at DESC");
        
        if let Some(limit) = filter.limit {
            query.push_str(" LIMIT ?");
            bindings.push(limit.to_string());
        }
        
        if let Some(offset) = filter.offset {
            query.push_str(" OFFSET ?");
            bindings.push(offset.to_string());
        }
        
        // Execute query with dynamic bindings
        let mut q = sqlx::query(&query);
        for binding in bindings {
            q = q.bind(binding);
        }
        
        let rows = q.fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(|row| self.row_to_user(row)).collect())
    }
}

impl SqliteUserStore {
    fn row_to_user(&self, row: sqlx::sqlite::SqliteRow) -> User {
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

// Also implement ApiKeyStore trait
#[async_trait]
impl crate::ApiKeyStore for SqliteUserStore {
    async fn create_api_key(&self, request: crate::api_keys::CreateApiKeyRequest) -> Result<(crate::ApiKey, String)> {
        use rand::Rng;
        use sha2::{Sha256, Digest};
        
        // Generate the actual API key
        let raw_key = format!("rvoip_ak_live_{}", 
            rand::thread_rng()
                .sample_iter(&rand::distributions::Alphanumeric)
                .take(32)
                .map(char::from)
                .collect::<String>()
        );
        
        // Hash the key for storage
        let mut hasher = Sha256::new();
        hasher.update(raw_key.as_bytes());
        let key_hash = format!("{:x}", hasher.finalize());
        
        let api_key = crate::ApiKey {
            id: crate::User::new_id(),
            user_id: request.user_id,
            name: request.name,
            key_hash: key_hash.clone(),
            permissions: request.permissions,
            expires_at: request.expires_at,
            last_used: None,
            created_at: Utc::now(),
        };
        
        let permissions_json = serde_json::to_string(&api_key.permissions).unwrap();
        
        sqlx::query(
            "INSERT INTO api_keys (id, user_id, name, key_hash, permissions, expires_at, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(&api_key.id)
        .bind(&api_key.user_id)
        .bind(&api_key.name)
        .bind(&api_key.key_hash)
        .bind(&permissions_json)
        .bind(&api_key.expires_at)
        .bind(&api_key.created_at)
        .execute(&self.pool)
        .await?;
        
        Ok((api_key, raw_key))
    }
    
    async fn validate_api_key(&self, key: &str) -> Result<Option<crate::ApiKey>> {
        use sha2::{Sha256, Digest};
        
        // Hash the provided key
        let mut hasher = Sha256::new();
        hasher.update(key.as_bytes());
        let key_hash = format!("{:x}", hasher.finalize());
        
        let row = sqlx::query(
            "SELECT id, user_id, name, key_hash, permissions, expires_at, last_used, created_at
             FROM api_keys WHERE key_hash = ?"
        )
        .bind(&key_hash)
        .fetch_optional(&self.pool)
        .await?;
        
        if let Some(row) = row {
            let mut api_key = crate::ApiKey {
                id: row.get("id"),
                user_id: row.get("user_id"),
                name: row.get("name"),
                key_hash: row.get("key_hash"),
                permissions: serde_json::from_str(row.get("permissions")).unwrap_or_default(),
                expires_at: row.get("expires_at"),
                last_used: row.get("last_used"),
                created_at: row.get("created_at"),
            };
            
            // Check if expired
            if let Some(expires_at) = api_key.expires_at {
                if expires_at < Utc::now() {
                    return Err(Error::ApiKeyExpired);
                }
            }
            
            // Update last_used
            let now = Utc::now();
            sqlx::query("UPDATE api_keys SET last_used = ? WHERE id = ?")
                .bind(&now)
                .bind(&api_key.id)
                .execute(&self.pool)
                .await?;
            
            // Update the returned object to reflect the change
            api_key.last_used = Some(now);
            
            Ok(Some(api_key))
        } else {
            Ok(None)
        }
    }
    
    async fn revoke_api_key(&self, id: &str) -> Result<()> {
        let result = sqlx::query("DELETE FROM api_keys WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        
        if result.rows_affected() == 0 {
            return Err(Error::ApiKeyNotFound);
        }
        
        Ok(())
    }
    
    async fn list_api_keys(&self, user_id: &str) -> Result<Vec<crate::ApiKey>> {
        let rows = sqlx::query(
            "SELECT id, user_id, name, key_hash, permissions, expires_at, last_used, created_at
             FROM api_keys WHERE user_id = ? ORDER BY created_at DESC"
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        
        Ok(rows.into_iter().map(|row| crate::ApiKey {
            id: row.get("id"),
            user_id: row.get("user_id"),
            name: row.get("name"),
            key_hash: row.get("key_hash"),
            permissions: serde_json::from_str(row.get("permissions")).unwrap_or_default(),
            expires_at: row.get("expires_at"),
            last_used: row.get("last_used"),
            created_at: row.get("created_at"),
        }).collect())
    }
}
