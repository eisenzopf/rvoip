//! Knowledge base endpoints — articles and talk scripts.

use axum::{Router, routing::{get, put, post, delete}, extract::{State, Path, Query}, Json};
use serde::{Deserialize, Serialize};
use rvoip_call_engine::database::sqlx::Row;

use crate::auth::{AuthUser, require_role, ROLE_ADMIN, ROLE_SUPER_ADMIN};
use crate::error::{ApiResponse, ConsoleError, ConsoleResult};
use crate::server::AppState;

// -- Types --------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ArticleView {
    pub id: String,
    pub title: String,
    pub category: Option<String>,
    pub content: String,
    pub tags: Option<String>,
    pub is_published: bool,
    pub view_count: i64,
    pub created_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct ScriptView {
    pub id: String,
    pub name: String,
    pub scenario: Option<String>,
    pub content: String,
    pub category: Option<String>,
    pub is_active: bool,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateArticleBody {
    pub title: String,
    pub category: Option<String>,
    pub content: String,
    pub tags: Option<String>,
    pub is_published: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateArticleBody {
    pub title: Option<String>,
    pub category: Option<String>,
    pub content: Option<String>,
    pub tags: Option<String>,
    pub is_published: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CreateScriptBody {
    pub name: String,
    pub scenario: Option<String>,
    pub content: String,
    pub category: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateScriptBody {
    pub name: Option<String>,
    pub scenario: Option<String>,
    pub content: Option<String>,
    pub category: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ArticleQuery {
    pub category: Option<String>,
    pub search: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ScriptQuery {
    pub category: Option<String>,
}

fn rid() -> String { uuid::Uuid::new_v4().to_string() }

// -- Database init ------------------------------------------------------------

/// Ensure knowledge tables exist and seed defaults.
pub async fn init_knowledge_tables(state: &AppState) -> Result<(), String> {
    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(()),
    };

    // Create knowledge_articles table
    rvoip_call_engine::database::sqlx::query(
        "CREATE TABLE IF NOT EXISTS knowledge_articles (\
            id TEXT PRIMARY KEY, \
            title TEXT NOT NULL, \
            category TEXT, \
            content TEXT NOT NULL, \
            tags TEXT, \
            is_published BOOLEAN DEFAULT FALSE, \
            view_count INTEGER DEFAULT 0, \
            created_by TEXT, \
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(), \
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()\
        )",
    )
    .execute(db.pool())
    .await
    .map_err(|e| format!("failed to create knowledge_articles table: {e}"))?;

    // Create talk_scripts table
    rvoip_call_engine::database::sqlx::query(
        "CREATE TABLE IF NOT EXISTS talk_scripts (\
            id TEXT PRIMARY KEY, \
            name TEXT NOT NULL, \
            scenario TEXT, \
            content TEXT NOT NULL, \
            category TEXT, \
            is_active BOOLEAN DEFAULT TRUE, \
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()\
        )",
    )
    .execute(db.pool())
    .await
    .map_err(|e| format!("failed to create talk_scripts table: {e}"))?;

    // Seed default articles (idempotent)
    let articles: &[(&str, &str, Option<&str>, &str, Option<&str>, bool)] = &[
        (
            "ART-001",
            "产品介绍",
            Some("产品"),
            "本公司提供全方位的VoIP通信解决方案，包括SIP语音服务、呼叫中心系统、\
             智能IVR导航等核心产品。我们的平台基于纯Rust技术栈构建，具备高性能、\
             低延迟的特点，支持大规模并发通话处理。",
            Some("产品,VoIP,SIP"),
            true,
        ),
        (
            "ART-002",
            "故障排除指南",
            Some("技术支持"),
            "常见故障排除步骤：\n\n1. 通话无声音：检查网络连接、防火墙设置，确认RTP端口开放。\n\
             2. 注册失败：验证SIP账号密码，检查服务器地址和端口配置。\n\
             3. 通话中断：排查网络抖动和丢包，建议使用QoS策略。\n\
             4. 回声问题：启用AEC（声学回声消除），调整音频设备设置。",
            Some("故障,排除,技术支持"),
            true,
        ),
        (
            "ART-003",
            "退款流程说明",
            Some("客服"),
            "退款流程：\n\n1. 客户提交退款申请，说明退款原因。\n\
             2. 客服确认订单信息，核实退款条件。\n\
             3. 提交至财务部门审核（1-3个工作日）。\n\
             4. 审核通过后原路返回，预计3-5个工作日到账。\n\n\
             注意：超过30天的订单不支持退款。",
            Some("退款,流程,客服"),
            false,
        ),
    ];

    for (id, title, category, content, tags, published) in articles {
        let _ = rvoip_call_engine::database::sqlx::query(
            "INSERT INTO knowledge_articles (id, title, category, content, tags, is_published) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(title)
        .bind(category)
        .bind(content)
        .bind(tags)
        .bind(published)
        .execute(db.pool())
        .await;
    }

    // Seed default talk scripts (idempotent)
    let scripts: &[(&str, &str, Option<&str>, &str, Option<&str>, bool)] = &[
        (
            "SCR-001",
            "客户投诉话术",
            Some("投诉处理"),
            "您好，非常抱歉给您带来不好的体验。请您详细描述一下遇到的问题，\
             我会认真记录并尽快为您解决。\n\n\
             [倾听客户描述]\n\n\
             我理解您的感受，这个问题确实给您造成了困扰。我现在就为您查询处理，\
             请您稍等片刻。\n\n\
             [处理完毕后]\n\n\
             问题已经为您处理好了，请问还有其他需要帮助的吗？",
            Some("投诉"),
            true,
        ),
        (
            "SCR-002",
            "产品咨询话术",
            Some("产品咨询"),
            "您好，感谢您对我们产品的关注！请问您想了解哪方面的信息呢？\n\n\
             [根据客户需求介绍]\n\n\
             我们的产品主要有以下优势：\n\
             1. 高可靠性：99.99%可用性保障\n\
             2. 灵活部署：支持云端和本地部署\n\
             3. 丰富功能：包含IVR、ACD、录音等全套功能\n\n\
             如果您感兴趣，我可以为您安排一次免费的产品演示。",
            Some("咨询"),
            true,
        ),
        (
            "SCR-003",
            "开场白标准话术",
            Some("通用"),
            "您好，这里是[公司名称]客户服务中心，我是客服[工号]号，很高兴为您服务。\
             请问有什么可以帮您的？\n\n\
             [如需转接]\n\
             请您稍等，我为您转接到专业的[部门]同事，请不要挂断。\n\n\
             [结束语]\n\
             感谢您的来电，祝您生活愉快，再见！",
            Some("通用"),
            true,
        ),
    ];

    for (id, name, scenario, content, category, active) in scripts {
        let _ = rvoip_call_engine::database::sqlx::query(
            "INSERT INTO talk_scripts (id, name, scenario, content, category, is_active) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(name)
        .bind(scenario)
        .bind(content)
        .bind(category)
        .bind(active)
        .execute(db.pool())
        .await;
    }

    Ok(())
}

// -- Article Handlers ---------------------------------------------------------

async fn list_articles(
    State(state): State<AppState>,
    _auth: AuthUser,
    Query(params): Query<ArticleQuery>,
) -> ConsoleResult<Json<ApiResponse<Vec<ArticleView>>>> {
    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(Json(ApiResponse::success(Vec::new(), rid()))),
    };

    // Build dynamic query
    let mut sql = String::from(
        "SELECT id, title, category, content, tags, is_published, view_count, \
         created_by, created_at, updated_at FROM knowledge_articles WHERE 1=1",
    );
    let mut bind_idx: u32 = 0;
    let mut binds: Vec<String> = Vec::new();

    if let Some(ref cat) = params.category {
        if !cat.is_empty() {
            bind_idx += 1;
            sql.push_str(&format!(" AND category = ${bind_idx}"));
            binds.push(cat.clone());
        }
    }
    if let Some(ref search) = params.search {
        if !search.is_empty() {
            bind_idx += 1;
            let si = bind_idx;
            bind_idx += 1;
            let ti = bind_idx;
            sql.push_str(&format!(" AND (title ILIKE ${si} OR content ILIKE ${ti})"));
            let pattern = format!("%{search}%");
            binds.push(pattern.clone());
            binds.push(pattern);
        }
    }
    sql.push_str(" ORDER BY updated_at DESC");

    let mut query = rvoip_call_engine::database::sqlx::query(&sql);
    for b in &binds {
        query = query.bind(b);
    }

    let rows = query
        .fetch_all(db.pool())
        .await
        .map_err(|e| ConsoleError::Internal(format!("articles query failed: {e}")))?;

    let articles: Vec<ArticleView> = rows.iter().map(|row| ArticleView {
        id: row.try_get("id").unwrap_or_default(),
        title: row.try_get("title").unwrap_or_default(),
        category: row.try_get("category").unwrap_or_default(),
        content: row.try_get("content").unwrap_or_default(),
        tags: row.try_get("tags").unwrap_or_default(),
        is_published: row.try_get("is_published").unwrap_or_default(),
        view_count: row.try_get::<i32, _>("view_count").unwrap_or_default() as i64,
        created_by: row.try_get("created_by").unwrap_or_default(),
        created_at: row.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
            .map(|d| d.to_rfc3339())
            .unwrap_or_default(),
        updated_at: row.try_get::<chrono::DateTime<chrono::Utc>, _>("updated_at")
            .map(|d| d.to_rfc3339())
            .unwrap_or_default(),
    }).collect();

    Ok(Json(ApiResponse::success(articles, rid())))
}

async fn create_article(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateArticleBody>,
) -> ConsoleResult<Json<ApiResponse<ArticleView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    if body.title.is_empty() || body.content.is_empty() {
        return Err(ConsoleError::BadRequest("title and content are required".into()));
    }

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    // Generate next ART-NNN ID
    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT COUNT(*) AS cnt FROM knowledge_articles",
    )
    .fetch_one(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("count articles failed: {e}")))?;

    let count: i64 = row.try_get("cnt").unwrap_or_default();
    let id = format!("ART-{:03}", count + 1);
    let published = body.is_published.unwrap_or(false);

    rvoip_call_engine::database::sqlx::query(
        "INSERT INTO knowledge_articles (id, title, category, content, tags, is_published, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(&id)
    .bind(&body.title)
    .bind(&body.category)
    .bind(&body.content)
    .bind(&body.tags)
    .bind(published)
    .bind(&auth.username)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("insert article failed: {e}")))?;

    let view = ArticleView {
        id,
        title: body.title,
        category: body.category,
        content: body.content,
        tags: body.tags,
        is_published: published,
        view_count: 0,
        created_by: Some(auth.username),
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    };

    Ok(Json(ApiResponse::success(view, rid())))
}

async fn update_article(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(article_id): Path<String>,
    Json(body): Json<UpdateArticleBody>,
) -> ConsoleResult<Json<ApiResponse<ArticleView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let current = rvoip_call_engine::database::sqlx::query(
        "SELECT id, title, category, content, tags, is_published, view_count, created_by, created_at \
         FROM knowledge_articles WHERE id = $1",
    )
    .bind(&article_id)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("fetch article failed: {e}")))?
    .ok_or_else(|| ConsoleError::NotFound(format!("article {article_id}")))?;

    let title: String = body.title.unwrap_or_else(|| current.try_get("title").unwrap_or_default());
    let category: Option<String> = body.category.or_else(|| current.try_get("category").unwrap_or_default());
    let content: String = body.content.unwrap_or_else(|| current.try_get("content").unwrap_or_default());
    let tags: Option<String> = body.tags.or_else(|| current.try_get("tags").unwrap_or_default());
    let is_published: bool = body.is_published.unwrap_or_else(|| current.try_get("is_published").unwrap_or_default());

    rvoip_call_engine::database::sqlx::query(
        "UPDATE knowledge_articles SET title = $1, category = $2, content = $3, tags = $4, \
         is_published = $5, updated_at = NOW() WHERE id = $6",
    )
    .bind(&title)
    .bind(&category)
    .bind(&content)
    .bind(&tags)
    .bind(is_published)
    .bind(&article_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("update article failed: {e}")))?;

    let view_count: i32 = current.try_get("view_count").unwrap_or_default();
    let created_by: Option<String> = current.try_get("created_by").unwrap_or_default();
    let created_at: String = current.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
        .map(|d| d.to_rfc3339())
        .unwrap_or_default();

    let view = ArticleView {
        id: article_id,
        title,
        category,
        content,
        tags,
        is_published,
        view_count: view_count as i64,
        created_by,
        created_at,
        updated_at: chrono::Utc::now().to_rfc3339(),
    };

    Ok(Json(ApiResponse::success(view, rid())))
}

async fn delete_article(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(article_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let result = rvoip_call_engine::database::sqlx::query(
        "DELETE FROM knowledge_articles WHERE id = $1",
    )
    .bind(&article_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("delete article failed: {e}")))?;

    if result.rows_affected() == 0 {
        return Err(ConsoleError::NotFound(format!("article {article_id}")));
    }

    Ok(Json(ApiResponse::success(format!("article {article_id} deleted"), rid())))
}

async fn view_article(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(article_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let result = rvoip_call_engine::database::sqlx::query(
        "UPDATE knowledge_articles SET view_count = view_count + 1 WHERE id = $1",
    )
    .bind(&article_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("increment view_count failed: {e}")))?;

    if result.rows_affected() == 0 {
        return Err(ConsoleError::NotFound(format!("article {article_id}")));
    }

    Ok(Json(ApiResponse::success("ok".to_string(), rid())))
}

// -- Script Handlers ----------------------------------------------------------

async fn list_scripts(
    State(state): State<AppState>,
    _auth: AuthUser,
    Query(params): Query<ScriptQuery>,
) -> ConsoleResult<Json<ApiResponse<Vec<ScriptView>>>> {
    let db = match state.engine.database_manager() {
        Some(db) => db,
        None => return Ok(Json(ApiResponse::success(Vec::new(), rid()))),
    };

    let (sql, bind_val) = if let Some(ref cat) = params.category {
        if !cat.is_empty() {
            (
                "SELECT id, name, scenario, content, category, is_active, created_at \
                 FROM talk_scripts WHERE category = $1 ORDER BY created_at DESC".to_string(),
                Some(cat.clone()),
            )
        } else {
            (
                "SELECT id, name, scenario, content, category, is_active, created_at \
                 FROM talk_scripts ORDER BY created_at DESC".to_string(),
                None,
            )
        }
    } else {
        (
            "SELECT id, name, scenario, content, category, is_active, created_at \
             FROM talk_scripts ORDER BY created_at DESC".to_string(),
            None,
        )
    };

    let mut query = rvoip_call_engine::database::sqlx::query(&sql);
    if let Some(ref val) = bind_val {
        query = query.bind(val);
    }

    let rows = query
        .fetch_all(db.pool())
        .await
        .map_err(|e| ConsoleError::Internal(format!("scripts query failed: {e}")))?;

    let scripts: Vec<ScriptView> = rows.iter().map(|row| ScriptView {
        id: row.try_get("id").unwrap_or_default(),
        name: row.try_get("name").unwrap_or_default(),
        scenario: row.try_get("scenario").unwrap_or_default(),
        content: row.try_get("content").unwrap_or_default(),
        category: row.try_get("category").unwrap_or_default(),
        is_active: row.try_get("is_active").unwrap_or_default(),
        created_at: row.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
            .map(|d| d.to_rfc3339())
            .unwrap_or_default(),
    }).collect();

    Ok(Json(ApiResponse::success(scripts, rid())))
}

async fn create_script(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateScriptBody>,
) -> ConsoleResult<Json<ApiResponse<ScriptView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    if body.name.is_empty() || body.content.is_empty() {
        return Err(ConsoleError::BadRequest("name and content are required".into()));
    }

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let row = rvoip_call_engine::database::sqlx::query(
        "SELECT COUNT(*) AS cnt FROM talk_scripts",
    )
    .fetch_one(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("count scripts failed: {e}")))?;

    let count: i64 = row.try_get("cnt").unwrap_or_default();
    let id = format!("SCR-{:03}", count + 1);
    let is_active = body.is_active.unwrap_or(true);

    rvoip_call_engine::database::sqlx::query(
        "INSERT INTO talk_scripts (id, name, scenario, content, category, is_active) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(&id)
    .bind(&body.name)
    .bind(&body.scenario)
    .bind(&body.content)
    .bind(&body.category)
    .bind(is_active)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("insert script failed: {e}")))?;

    let view = ScriptView {
        id,
        name: body.name,
        scenario: body.scenario,
        content: body.content,
        category: body.category,
        is_active,
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    Ok(Json(ApiResponse::success(view, rid())))
}

async fn update_script(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(script_id): Path<String>,
    Json(body): Json<UpdateScriptBody>,
) -> ConsoleResult<Json<ApiResponse<ScriptView>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let current = rvoip_call_engine::database::sqlx::query(
        "SELECT id, name, scenario, content, category, is_active, created_at \
         FROM talk_scripts WHERE id = $1",
    )
    .bind(&script_id)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("fetch script failed: {e}")))?
    .ok_or_else(|| ConsoleError::NotFound(format!("script {script_id}")))?;

    let name: String = body.name.unwrap_or_else(|| current.try_get("name").unwrap_or_default());
    let scenario: Option<String> = body.scenario.or_else(|| current.try_get("scenario").unwrap_or_default());
    let content: String = body.content.unwrap_or_else(|| current.try_get("content").unwrap_or_default());
    let category: Option<String> = body.category.or_else(|| current.try_get("category").unwrap_or_default());
    let is_active: bool = body.is_active.unwrap_or_else(|| current.try_get("is_active").unwrap_or_default());

    rvoip_call_engine::database::sqlx::query(
        "UPDATE talk_scripts SET name = $1, scenario = $2, content = $3, category = $4, \
         is_active = $5 WHERE id = $6",
    )
    .bind(&name)
    .bind(&scenario)
    .bind(&content)
    .bind(&category)
    .bind(is_active)
    .bind(&script_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("update script failed: {e}")))?;

    let created_at: String = current.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
        .map(|d| d.to_rfc3339())
        .unwrap_or_default();

    let view = ScriptView {
        id: script_id,
        name,
        scenario,
        content,
        category,
        is_active,
        created_at,
    };

    Ok(Json(ApiResponse::success(view, rid())))
}

async fn delete_script(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(script_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<String>>> {
    require_role(&auth, &[ROLE_ADMIN, ROLE_SUPER_ADMIN])?;

    let db = state.engine.database_manager()
        .ok_or_else(|| ConsoleError::Internal("database not configured".into()))?;

    let result = rvoip_call_engine::database::sqlx::query(
        "DELETE FROM talk_scripts WHERE id = $1",
    )
    .bind(&script_id)
    .execute(db.pool())
    .await
    .map_err(|e| ConsoleError::Internal(format!("delete script failed: {e}")))?;

    if result.rows_affected() == 0 {
        return Err(ConsoleError::NotFound(format!("script {script_id}")));
    }

    Ok(Json(ApiResponse::success(format!("script {script_id} deleted"), rid())))
}

// -- Router -------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/articles", get(list_articles).post(create_article))
        .route("/articles/{id}", put(update_article).delete(delete_article))
        .route("/articles/{id}/view", post(view_article))
        .route("/scripts", get(list_scripts).post(create_script))
        .route("/scripts/{id}", put(update_script).delete(delete_script))
}
