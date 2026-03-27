//! Reports center endpoints — aggregate data from call_records, agents, queues.

use axum::{Router, routing::get, extract::{State, Query}, Json};
use serde::{Deserialize, Serialize};
use rvoip_call_engine::database::sqlx::Row;

use crate::auth::{AuthUser, require_role, ROLE_SUPERVISOR, ROLE_ADMIN, ROLE_SUPER_ADMIN};
use crate::error::{ApiResponse, ConsoleError, ConsoleResult};
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct DailyReport {
    pub date: String,
    pub total_calls: i64,
    pub answered_calls: i64,
    pub abandoned_calls: i64,
    pub avg_duration_seconds: f64,
    pub avg_wait_seconds: f64,
    pub sla_percentage: f64,
}

#[derive(Debug, Serialize)]
pub struct AgentPerformanceReport {
    pub agent_id: String,
    pub agent_name: String,
    pub total_calls: i64,
    pub avg_duration_seconds: f64,
    pub total_duration_seconds: i64,
}

#[derive(Debug, Serialize)]
pub struct QueuePerformanceReport {
    pub queue_name: String,
    pub total_calls: i64,
    pub avg_wait_seconds: f64,
    pub max_wait_seconds: i64,
    pub abandoned: i64,
}

#[derive(Debug, Serialize)]
pub struct SummaryReport {
    pub period: String,
    pub total_calls: i64,
    pub total_agents: i64,
    pub avg_calls_per_agent: f64,
    pub avg_duration: f64,
    pub busiest_hour: String,
    pub top_agents: Vec<AgentPerformanceReport>,
    pub queue_stats: Vec<QueuePerformanceReport>,
}

// ---------------------------------------------------------------------------
// Query params
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct DailyQuery {
    pub date: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RangeQuery {
    pub start: Option<String>,
    pub end: Option<String>,
    pub agent_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn rid() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

// ---------------------------------------------------------------------------
// GET /reports/daily?date=2026-03-24
// ---------------------------------------------------------------------------

async fn daily_report(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<DailyQuery>,
) -> ConsoleResult<Json<ApiResponse<DailyReport>>> {
    require_role(&auth, &[ROLE_SUPERVISOR, ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let date = params.date.unwrap_or_else(today);

    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => {
            return Ok(Json(ApiResponse::success(
                DailyReport {
                    date: date.clone(),
                    total_calls: 0,
                    answered_calls: 0,
                    abandoned_calls: 0,
                    avg_duration_seconds: 0.0,
                    avg_wait_seconds: 0.0,
                    sla_percentage: 0.0,
                },
                rid(),
            )));
        }
    };

    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT \
            COUNT(*) as total, \
            COUNT(CASE WHEN disposition = 'answered' THEN 1 END) as answered, \
            COUNT(CASE WHEN disposition = 'abandoned' THEN 1 END) as abandoned, \
            CAST(COALESCE(AVG(duration_seconds), 0) AS float8) as avg_duration \
         FROM call_records \
         WHERE start_time::date = $1::date",
    )
    .bind(&date)
    .fetch_one(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("daily report query failed: {e}")))?;

    let total: i64 = row.try_get("total").unwrap_or_default();
    let answered: i64 = row.try_get("answered").unwrap_or_default();
    let abandoned: i64 = row.try_get("abandoned").unwrap_or_default();
    let avg_dur: f64 = row.try_get("avg_duration").unwrap_or_default();

    let sla_percentage = if total > 0 {
        (answered as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    Ok(Json(ApiResponse::success(
        DailyReport {
            date,
            total_calls: total,
            answered_calls: answered,
            abandoned_calls: abandoned,
            avg_duration_seconds: avg_dur,
            avg_wait_seconds: 0.0,
            sla_percentage,
        },
        rid(),
    )))
}

// ---------------------------------------------------------------------------
// GET /reports/agent-performance?start=&end=&agent_id=
// ---------------------------------------------------------------------------

async fn agent_performance(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<RangeQuery>,
) -> ConsoleResult<Json<ApiResponse<Vec<AgentPerformanceReport>>>> {
    require_role(&auth, &[ROLE_SUPERVISOR, ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let start = params.start.unwrap_or_else(today);
    let end = params.end.unwrap_or_else(today);

    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => {
            return Ok(Json(ApiResponse::success(Vec::new(), rid())));
        }
    };

    let rows = if let Some(ref aid) = params.agent_id {
        rvoip_call_engine::database::sqlx::query(
            "SELECT agent_id, \
                    COUNT(*) as total_calls, \
                    CAST(COALESCE(AVG(duration_seconds), 0) AS float8) as avg_duration, \
                    COALESCE(SUM(duration_seconds), 0)::bigint as total_duration \
             FROM call_records \
             WHERE start_time::date >= $1::date AND start_time::date <= $2::date \
               AND agent_id = $3 \
             GROUP BY agent_id \
             ORDER BY total_calls DESC",
        )
        .bind(&start)
        .bind(&end)
        .bind(aid)
        .fetch_all(db.pool())
        .await
    } else {
        rvoip_call_engine::database::sqlx::query(
            "SELECT agent_id, \
                    COUNT(*) as total_calls, \
                    CAST(COALESCE(AVG(duration_seconds), 0) AS float8) as avg_duration, \
                    COALESCE(SUM(duration_seconds), 0)::bigint as total_duration \
             FROM call_records \
             WHERE start_time::date >= $1::date AND start_time::date <= $2::date \
               AND agent_id IS NOT NULL \
             GROUP BY agent_id \
             ORDER BY total_calls DESC",
        )
        .bind(&start)
        .bind(&end)
        .fetch_all(db.pool())
        .await
    }
    .map_err(|e| ConsoleError::Internal(format!("agent performance query failed: {e}")))?;

    let entries: Vec<AgentPerformanceReport> = rows
        .iter()
        .map(|row| {
            let agent_id: String = row.try_get("agent_id").unwrap_or_default();
            AgentPerformanceReport {
                agent_name: agent_id.clone(),
                agent_id,
                total_calls: row.try_get("total_calls").unwrap_or_default(),
                avg_duration_seconds: row.try_get("avg_duration").unwrap_or_default(),
                total_duration_seconds: row.try_get("total_duration").unwrap_or_default(),
            }
        })
        .collect();

    Ok(Json(ApiResponse::success(entries, rid())))
}

// ---------------------------------------------------------------------------
// GET /reports/queue-performance?start=&end=
// ---------------------------------------------------------------------------

async fn queue_performance(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<RangeQuery>,
) -> ConsoleResult<Json<ApiResponse<Vec<QueuePerformanceReport>>>> {
    require_role(&auth, &[ROLE_SUPERVISOR, ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let start = params.start.unwrap_or_else(today);
    let end = params.end.unwrap_or_else(today);

    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => {
            return Ok(Json(ApiResponse::success(Vec::new(), rid())));
        }
    };

    let rows = rvoip_call_engine::database::sqlx::query(
        "SELECT queue_name, \
                COUNT(*) as total_calls, \
                CAST(COALESCE(AVG(duration_seconds), 0) AS float8) as avg_wait, \
                COALESCE(MAX(duration_seconds), 0)::bigint as max_wait, \
                COUNT(CASE WHEN disposition = 'abandoned' THEN 1 END) as abandoned \
         FROM call_records \
         WHERE start_time::date >= $1::date AND start_time::date <= $2::date \
           AND queue_name IS NOT NULL \
         GROUP BY queue_name \
         ORDER BY total_calls DESC",
    )
    .bind(&start)
    .bind(&end)
    .fetch_all(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("queue performance query failed: {e}")))?;

    let entries: Vec<QueuePerformanceReport> = rows
        .iter()
        .map(|row| {
            QueuePerformanceReport {
                queue_name: row.try_get("queue_name").unwrap_or_default(),
                total_calls: row.try_get("total_calls").unwrap_or_default(),
                avg_wait_seconds: row.try_get("avg_wait").unwrap_or_default(),
                max_wait_seconds: row.try_get("max_wait").unwrap_or_default(),
                abandoned: row.try_get("abandoned").unwrap_or_default(),
            }
        })
        .collect();

    Ok(Json(ApiResponse::success(entries, rid())))
}

// ---------------------------------------------------------------------------
// GET /reports/summary?start=&end=
// ---------------------------------------------------------------------------

async fn summary_report(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<RangeQuery>,
) -> ConsoleResult<Json<ApiResponse<SummaryReport>>> {
    require_role(&auth, &[ROLE_SUPERVISOR, ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let start = params.start.unwrap_or_else(today);
    let end = params.end.unwrap_or_else(today);

    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => {
            return Ok(Json(ApiResponse::success(
                SummaryReport {
                    period: format!("{start} ~ {end}"),
                    total_calls: 0,
                    total_agents: 0,
                    avg_calls_per_agent: 0.0,
                    avg_duration: 0.0,
                    busiest_hour: "--".to_string(),
                    top_agents: Vec::new(),
                    queue_stats: Vec::new(),
                },
                rid(),
            )));
        }
    };

    // Overall stats
    let overview = rvoip_call_engine::database::sqlx::query(
        "SELECT COUNT(*) as total_calls, \
                COUNT(DISTINCT agent_id) as total_agents, \
                CAST(COALESCE(AVG(duration_seconds), 0) AS float8) as avg_duration \
         FROM call_records \
         WHERE start_time::date >= $1::date AND start_time::date <= $2::date",
    )
    .bind(&start)
    .bind(&end)
    .fetch_one(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("summary overview query failed: {e}")))?;

    let total_calls: i64 = overview.try_get("total_calls").unwrap_or_default();
    let total_agents: i64 = overview.try_get("total_agents").unwrap_or_default();
    let avg_duration: f64 = overview.try_get("avg_duration").unwrap_or_default();

    let avg_calls_per_agent = if total_agents > 0 {
        total_calls as f64 / total_agents as f64
    } else {
        0.0
    };

    // Busiest hour
    let busiest_row = rvoip_call_engine::database::sqlx::query(
        "SELECT EXTRACT(HOUR FROM start_time)::int as hr, COUNT(*) as cnt \
         FROM call_records \
         WHERE start_time::date >= $1::date AND start_time::date <= $2::date \
           AND start_time IS NOT NULL \
         GROUP BY hr ORDER BY cnt DESC LIMIT 1",
    )
    .bind(&start)
    .bind(&end)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("busiest hour query failed: {e}")))?;

    let busiest_hour = busiest_row
        .and_then(|r| {
            r.try_get::<i32, _>("hr")
                .ok()
                .map(|h| format!("{:02}:00", h))
        })
        .unwrap_or_else(|| "--".to_string());

    // Top agents (limit 10)
    let agent_rows = rvoip_call_engine::database::sqlx::query(
        "SELECT agent_id, \
                COUNT(*) as total_calls, \
                CAST(COALESCE(AVG(duration_seconds), 0) AS float8) as avg_duration, \
                COALESCE(SUM(duration_seconds), 0)::bigint as total_duration \
         FROM call_records \
         WHERE start_time::date >= $1::date AND start_time::date <= $2::date \
           AND agent_id IS NOT NULL \
         GROUP BY agent_id \
         ORDER BY total_calls DESC \
         LIMIT 10",
    )
    .bind(&start)
    .bind(&end)
    .fetch_all(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("top agents query failed: {e}")))?;

    let top_agents: Vec<AgentPerformanceReport> = agent_rows
        .iter()
        .map(|row| {
            let agent_id: String = row.try_get("agent_id").unwrap_or_default();
            AgentPerformanceReport {
                agent_name: agent_id.clone(),
                agent_id,
                total_calls: row.try_get("total_calls").unwrap_or_default(),
                avg_duration_seconds: row.try_get("avg_duration").unwrap_or_default(),
                total_duration_seconds: row.try_get("total_duration").unwrap_or_default(),
            }
        })
        .collect();

    // Queue stats
    let queue_rows = rvoip_call_engine::database::sqlx::query(
        "SELECT queue_name, \
                COUNT(*) as total_calls, \
                CAST(COALESCE(AVG(duration_seconds), 0) AS float8) as avg_wait, \
                COALESCE(MAX(duration_seconds), 0)::bigint as max_wait, \
                COUNT(CASE WHEN disposition = 'abandoned' THEN 1 END) as abandoned \
         FROM call_records \
         WHERE start_time::date >= $1::date AND start_time::date <= $2::date \
           AND queue_name IS NOT NULL \
         GROUP BY queue_name \
         ORDER BY total_calls DESC",
    )
    .bind(&start)
    .bind(&end)
    .fetch_all(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("queue stats query failed: {e}")))?;

    let queue_stats: Vec<QueuePerformanceReport> = queue_rows
        .iter()
        .map(|row| {
            QueuePerformanceReport {
                queue_name: row.try_get("queue_name").unwrap_or_default(),
                total_calls: row.try_get("total_calls").unwrap_or_default(),
                avg_wait_seconds: row.try_get("avg_wait").unwrap_or_default(),
                max_wait_seconds: row.try_get("max_wait").unwrap_or_default(),
                abandoned: row.try_get("abandoned").unwrap_or_default(),
            }
        })
        .collect();

    Ok(Json(ApiResponse::success(
        SummaryReport {
            period: format!("{start} ~ {end}"),
            total_calls,
            total_agents,
            avg_calls_per_agent,
            avg_duration,
            busiest_hour,
            top_agents,
            queue_stats,
        },
        rid(),
    )))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/daily", get(daily_report))
        .route("/agent-performance", get(agent_performance))
        .route("/queue-performance", get(queue_performance))
        .route("/summary", get(summary_report))
}
