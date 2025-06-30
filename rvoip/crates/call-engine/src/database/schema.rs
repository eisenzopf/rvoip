//! Database schema (sqlx migrations)
//!
//! The database schema is now managed through sqlx migrations in the `migrations/` directory.
//! This eliminates the need for manual schema initialization code.

pub use super::DatabaseManager;

// Schema is automatically applied through sqlx::migrate! in DatabaseManager::new() 