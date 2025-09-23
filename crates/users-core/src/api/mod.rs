//! REST API for users-core

use axum::Router;
use std::sync::Arc;
use crate::AuthenticationService;

/// Create the REST API router
pub fn create_router(_auth_service: Arc<AuthenticationService>) -> Router {
    todo!("Implement REST API router")
}
