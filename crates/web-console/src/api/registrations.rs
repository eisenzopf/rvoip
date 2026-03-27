//! SIP registration endpoints.
//!
//! Integrates with registrar-core to expose current SIP registrations.

use axum::{Router, routing::get, extract::State, Json};
use serde::Serialize;

use crate::error::{ApiResponse, ConsoleResult};
use crate::server::AppState;

#[derive(Debug, Serialize)]
pub struct ContactView {
    pub uri: String,
    pub transport: String,
    pub user_agent: String,
    pub expires: String,
    pub q_value: f32,
}

#[derive(Debug, Serialize)]
pub struct RegistrationView {
    pub user_id: String,
    pub contacts: Vec<ContactView>,
    pub registered_at: String,
    pub expires: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct RegistrationsResponse {
    pub registrations: Vec<RegistrationView>,
    pub total: usize,
}

async fn list_registrations(
    State(state): State<AppState>,
) -> ConsoleResult<Json<ApiResponse<RegistrationsResponse>>> {
    let mut registrations = Vec::new();

    if let Some(ref registrar) = state.registrar {
        let user_ids = registrar.list_registered_users().await;

        for user_id in &user_ids {
            if let Ok(contacts) = registrar.lookup_user(user_id).await {
                let contact_views: Vec<ContactView> = contacts
                    .iter()
                    .map(|c| ContactView {
                        uri: c.uri.clone(),
                        transport: format!("{:?}", c.transport),
                        user_agent: c.user_agent.clone(),
                        expires: c.expires.to_rfc3339(),
                        q_value: c.q_value,
                    })
                    .collect();

                registrations.push(RegistrationView {
                    user_id: user_id.clone(),
                    contacts: contact_views,
                    registered_at: String::new(), // lookup_user returns ContactInfo, not full registration
                    expires: String::new(),
                    capabilities: vec![],
                });
            }
        }
    }

    let total = registrations.len();
    Ok(Json(ApiResponse::success(
        RegistrationsResponse { registrations, total },
        uuid::Uuid::new_v4().to_string(),
    )))
}

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(list_registrations))
}
