//! AI Copilot REST API endpoints.
//!
//! Provides text-based analysis, suggestion, and summarization endpoints
//! using rule-based logic (no external LLM dependency).

use axum::{Router, routing::{get, post}, extract::{State, Path}, Json};
use serde::{Deserialize, Serialize};
use rvoip_call_engine::database::sqlx::Row;

use crate::auth::AuthUser;
use crate::error::{ApiResponse, ConsoleError, ConsoleResult};
use crate::server::AppState;

// -- Types --------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AnalyzeRequest {
    pub text: String,
    pub call_id: Option<String>,
    pub agent_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AnalyzeResponse {
    pub intent: String,
    pub sentiment: f32,
    pub sentiment_label: String,
    pub suggestion: String,
    pub knowledge_refs: Vec<KnowledgeRef>,
    pub quality_items: Vec<QualityItem>,
}

#[derive(Debug, Serialize)]
pub struct KnowledgeRef {
    pub id: String,
    pub title: String,
    pub relevance: f32,
}

#[derive(Debug, Serialize)]
pub struct QualityItem {
    pub name: String,
    pub checked: bool,
}

#[derive(Debug, Deserialize)]
pub struct SuggestRequest {
    pub customer_text: String,
    pub context: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SuggestResponse {
    pub suggestion: String,
    pub scripts: Vec<ScriptRef>,
}

#[derive(Debug, Serialize)]
pub struct ScriptRef {
    pub name: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct SummarizeRequest {
    pub call_id: String,
    pub turns: Vec<TurnInput>,
}

#[derive(Debug, Deserialize)]
pub struct TurnInput {
    pub speaker: String,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct SummaryResponse {
    pub summary: String,
    pub key_topics: Vec<String>,
    pub overall_sentiment: String,
    pub quality_score: i32,
}

#[derive(Debug, Serialize)]
pub struct AiConfigResponse {
    pub enabled: bool,
    pub provider: String,
    pub features: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ConversationTurn {
    pub speaker: String,
    pub text: String,
    pub timestamp: String,
}

fn rid() -> String { uuid::Uuid::new_v4().to_string() }

// -- Handlers -----------------------------------------------------------------

/// POST /ai/analyze — Analyse customer text (intent + sentiment + suggestion).
async fn analyze(
    State(state): State<AppState>,
    _auth: AuthUser,
    Json(req): Json<AnalyzeRequest>,
) -> ConsoleResult<Json<ApiResponse<AnalyzeResponse>>> {
    if req.text.is_empty() {
        return Err(ConsoleError::BadRequest("text is required".into()));
    }

    // Simple keyword-based intent detection
    let text_lower = req.text.to_lowercase();
    let intent = if text_lower.contains("网络") || text_lower.contains("连不上") || text_lower.contains("断网") {
        "network_troubleshoot"
    } else if text_lower.contains("退款") || text_lower.contains("退钱") {
        "refund_request"
    } else if text_lower.contains("账单") || text_lower.contains("费用") {
        "billing_inquiry"
    } else if text_lower.contains("投诉") || text_lower.contains("不满") {
        "complaint"
    } else {
        "general_inquiry"
    };

    // Simple sentiment analysis
    let (sentiment, label) = if text_lower.contains("不满") || text_lower.contains("投诉") || text_lower.contains("气死") {
        (-0.7_f32, "angry")
    } else if text_lower.contains("着急") || text_lower.contains("已经") || text_lower.contains("两天") {
        (-0.3_f32, "frustrated")
    } else if text_lower.contains("谢谢") || text_lower.contains("好的") {
        (0.5_f32, "positive")
    } else {
        (0.0_f32, "neutral")
    };

    // Search knowledge base for matching articles
    let knowledge_refs = if let Some(db) = state.engine.database_manager() {
        let truncated_len = req.text.len().min(20);
        // Find a valid char boundary for the search substring
        let safe_end = (0..=truncated_len)
            .rev()
            .find(|&i| req.text.is_char_boundary(i))
            .unwrap_or(0);
        let search = format!("%{}%", &req.text[..safe_end]);
        let rows = rvoip_call_engine::database::sqlx::query(
            "SELECT id, title FROM knowledge_articles WHERE title ILIKE $1 OR content ILIKE $1 LIMIT 3",
        )
        .bind(&search)
        .fetch_all(db.pool())
        .await
        .unwrap_or_default();

        rows.iter().map(|row| {
            KnowledgeRef {
                id: row.try_get("id").unwrap_or_default(),
                title: row.try_get("title").unwrap_or_default(),
                relevance: 0.85,
            }
        }).collect()
    } else {
        vec![]
    };

    // Generate suggestion based on intent
    let suggestion = match intent {
        "network_troubleshoot" => {
            "\u{6211}\u{7406}\u{89e3}\u{60a8}\u{7684}\u{7f51}\u{7edc}\u{95ee}\u{9898}\u{7ed9}\u{60a8}\u{5e26}\u{6765}\u{4e86}\u{4e0d}\u{4fbf}\u{3002}\u{8ba9}\u{6211}\u{5e2e}\u{60a8}\u{6392}\u{67e5}\u{4e00}\u{4e0b}\u{ff0c}\u{8bf7}\u{5148}\u{786e}\u{8ba4}\u{8def}\u{7531}\u{5668}\u{7684}\u{7535}\u{6e90}\u{6307}\u{793a}\u{706f}\u{662f}\u{5426}\u{6b63}\u{5e38}\u{4eae}\u{8d77}\u{ff1f}"
        }
        "refund_request" => {
            "\u{6211}\u{4e86}\u{89e3}\u{60a8}\u{7684}\u{9000}\u{6b3e}\u{9700}\u{6c42}\u{3002}\u{8bf7}\u{63d0}\u{4f9b}\u{60a8}\u{7684}\u{8ba2}\u{5355}\u{53f7}\u{ff0c}\u{6211}\u{6765}\u{4e3a}\u{60a8}\u{67e5}\u{8be2}\u{9000}\u{6b3e}\u{6d41}\u{7a0b}\u{3002}"
        }
        "billing_inquiry" => {
            "\u{597d}\u{7684}\u{ff0c}\u{6211}\u{6765}\u{5e2e}\u{60a8}\u{67e5}\u{8be2}\u{8d26}\u{5355}\u{8be6}\u{60c5}\u{3002}\u{8bf7}\u{95ee}\u{60a8}\u{9700}\u{8981}\u{67e5}\u{8be2}\u{54ea}\u{4e2a}\u{6708}\u{4efd}\u{7684}\u{8d26}\u{5355}\u{ff1f}"
        }
        "complaint" => {
            "\u{975e}\u{5e38}\u{62b1}\u{6b49}\u{7ed9}\u{60a8}\u{5e26}\u{6765}\u{4e0d}\u{597d}\u{7684}\u{4f53}\u{9a8c}\u{3002}\u{6211}\u{4f1a}\u{8ba4}\u{771f}\u{8bb0}\u{5f55}\u{60a8}\u{7684}\u{53cd}\u{9988}\u{5e76}\u{5c3d}\u{5feb}\u{5904}\u{7406}\u{3002}\u{8bf7}\u{8be6}\u{7ec6}\u{63cf}\u{8ff0}\u{4e00}\u{4e0b}\u{95ee}\u{9898}\u{3002}"
        }
        _ => {
            "\u{597d}\u{7684}\u{ff0c}\u{6211}\u{6765}\u{5e2e}\u{60a8}\u{5904}\u{7406}\u{3002}\u{8bf7}\u{8be6}\u{7ec6}\u{8bf4}\u{660e}\u{60a8}\u{7684}\u{9700}\u{6c42}\u{3002}"
        }
    };

    Ok(Json(ApiResponse::success(AnalyzeResponse {
        intent: intent.to_string(),
        sentiment,
        sentiment_label: label.to_string(),
        suggestion: suggestion.to_string(),
        knowledge_refs,
        quality_items: vec![
            QualityItem { name: "\u{786e}\u{8ba4}\u{5ba2}\u{6237}\u{8eab}\u{4efd}".into(), checked: false },
            QualityItem { name: "\u{4f7f}\u{7528}\u{4e13}\u{4e1a}\u{7528}\u{8bed}".into(), checked: true },
            QualityItem { name: "\u{63d0}\u{4f9b}\u{89e3}\u{51b3}\u{65b9}\u{6848}".into(), checked: false },
        ],
    }, rid())))
}

/// POST /ai/suggest — Get talk script suggestion based on customer text.
async fn suggest(
    State(state): State<AppState>,
    _auth: AuthUser,
    Json(req): Json<SuggestRequest>,
) -> ConsoleResult<Json<ApiResponse<SuggestResponse>>> {
    if req.customer_text.is_empty() {
        return Err(ConsoleError::BadRequest("customer_text is required".into()));
    }

    let text_lower = req.customer_text.to_lowercase();

    // Determine which scenario keyword to search for
    let scenario_keyword = if text_lower.contains("投诉") || text_lower.contains("不满") {
        "投诉"
    } else if text_lower.contains("咨询") || text_lower.contains("产品") || text_lower.contains("了解") {
        "咨询"
    } else {
        "通用"
    };

    let mut scripts = Vec::new();
    if let Some(db) = state.engine.database_manager() {
        let search = format!("%{scenario_keyword}%");
        let rows = rvoip_call_engine::database::sqlx::query(
            "SELECT name, content FROM talk_scripts WHERE is_active = TRUE AND (scenario ILIKE $1 OR category ILIKE $1) LIMIT 3",
        )
        .bind(&search)
        .fetch_all(db.pool())
        .await
        .unwrap_or_default();

        for row in &rows {
            scripts.push(ScriptRef {
                name: row.try_get("name").unwrap_or_default(),
                content: row.try_get("content").unwrap_or_default(),
            });
        }
    }

    let suggestion = if scripts.is_empty() {
        format!("No matching scripts found for: {}", req.customer_text)
    } else {
        scripts.first().map(|s| s.content.clone()).unwrap_or_default()
    };

    Ok(Json(ApiResponse::success(SuggestResponse {
        suggestion,
        scripts,
    }, rid())))
}

/// POST /ai/summarize — Generate a basic call summary from conversation turns.
async fn summarize(
    _state: State<AppState>,
    _auth: AuthUser,
    Json(req): Json<SummarizeRequest>,
) -> ConsoleResult<Json<ApiResponse<SummaryResponse>>> {
    if req.turns.is_empty() {
        return Err(ConsoleError::BadRequest("turns must not be empty".into()));
    }

    // Build summary from turns
    let total_turns = req.turns.len();
    let speakers: Vec<&str> = req.turns.iter().map(|t| t.speaker.as_str()).collect::<std::collections::HashSet<_>>().into_iter().collect();
    let all_text: String = req.turns.iter().map(|t| t.text.as_str()).collect::<Vec<_>>().join(" ");

    // Extract simple key topics from text
    let topic_keywords = ["网络", "退款", "账单", "费用", "投诉", "产品", "服务", "故障", "技术"];
    let key_topics: Vec<String> = topic_keywords.iter()
        .filter(|kw| all_text.contains(**kw))
        .map(|kw| (*kw).to_string())
        .collect();

    // Simple overall sentiment from the combined text
    let text_lower = all_text.to_lowercase();
    let overall_sentiment = if text_lower.contains("不满") || text_lower.contains("投诉") {
        "negative"
    } else if text_lower.contains("谢谢") || text_lower.contains("满意") {
        "positive"
    } else {
        "neutral"
    };

    // Basic quality score (0-100)
    let quality_score = 70_i32 + (total_turns.min(10) as i32 * 2);

    let summary = format!(
        "Call {} with {} participants and {} turns. Topics discussed: {}.",
        req.call_id,
        speakers.len(),
        total_turns,
        if key_topics.is_empty() { "general inquiry".to_string() } else { key_topics.join(", ") },
    );

    Ok(Json(ApiResponse::success(SummaryResponse {
        summary,
        key_topics,
        overall_sentiment: overall_sentiment.to_string(),
        quality_score,
    }, rid())))
}

/// GET /ai/config — Return current AI configuration.
async fn config(
    _auth: AuthUser,
) -> ConsoleResult<Json<ApiResponse<AiConfigResponse>>> {
    Ok(Json(ApiResponse::success(AiConfigResponse {
        enabled: true,
        provider: "rule-based".to_string(),
        features: vec![
            "intent_detection".to_string(),
            "sentiment_analysis".to_string(),
            "knowledge_search".to_string(),
            "script_suggestion".to_string(),
            "call_summary".to_string(),
        ],
    }, rid())))
}

/// GET /ai/conversation/:call_id — Get conversation turns for a call (stub).
async fn conversation(
    _auth: AuthUser,
    Path(call_id): Path<String>,
) -> ConsoleResult<Json<ApiResponse<Vec<ConversationTurn>>>> {
    // Stub — no conversation_turns table yet. Return empty list.
    let _call_id = call_id;
    Ok(Json(ApiResponse::success(Vec::<ConversationTurn>::new(), rid())))
}

// -- Router -------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/analyze", post(analyze))
        .route("/suggest", post(suggest))
        .route("/summarize", post(summarize))
        .route("/config", get(config))
        .route("/conversation/{call_id}", get(conversation))
}
