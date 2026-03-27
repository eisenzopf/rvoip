use sqlx::PgPool;

/// Initialize AI-related database tables
pub async fn init_ai_tables(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS conversation_turns (
            id BIGSERIAL PRIMARY KEY,
            call_id TEXT NOT NULL,
            turn_index INTEGER NOT NULL,
            speaker TEXT NOT NULL,
            asr_text TEXT,
            asr_confidence REAL,
            intent TEXT,
            sentiment TEXT,
            sentiment_score REAL,
            ai_suggestion TEXT,
            tts_text TEXT,
            knowledge_refs TEXT,
            latency_ms INTEGER,
            audio_duration_ms INTEGER,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            UNIQUE(call_id, turn_index)
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS call_ai_summaries (
            id BIGSERIAL PRIMARY KEY,
            call_id TEXT NOT NULL UNIQUE,
            summary TEXT,
            key_topics TEXT,
            sentiment_arc TEXT,
            overall_sentiment TEXT,
            quality_score INTEGER,
            quality_details TEXT,
            improvement_suggestions TEXT,
            customer_satisfaction TEXT,
            resolution_status TEXT,
            model_used TEXT,
            processing_time_ms INTEGER,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS ai_config (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            description TEXT,
            updated_at TIMESTAMPTZ DEFAULT NOW()
        )",
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Async recorder -- writes conversation turns to DB without blocking the pipeline
pub struct ConversationRecorder {
    pool: PgPool,
}

impl ConversationRecorder {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn record_turn(
        &self,
        call_id: &str,
        turn_index: i32,
        speaker: &str,
        asr_text: Option<&str>,
        intent: Option<&str>,
        sentiment: Option<&str>,
        sentiment_score: Option<f32>,
        ai_suggestion: Option<&str>,
        latency_ms: Option<i32>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO conversation_turns \
                (call_id, turn_index, speaker, asr_text, intent, sentiment, sentiment_score, ai_suggestion, latency_ms) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
             ON CONFLICT (call_id, turn_index) DO UPDATE SET \
                asr_text = COALESCE(EXCLUDED.asr_text, conversation_turns.asr_text), \
                intent = COALESCE(EXCLUDED.intent, conversation_turns.intent), \
                sentiment = COALESCE(EXCLUDED.sentiment, conversation_turns.sentiment), \
                ai_suggestion = COALESCE(EXCLUDED.ai_suggestion, conversation_turns.ai_suggestion)",
        )
        .bind(call_id)
        .bind(turn_index)
        .bind(speaker)
        .bind(asr_text)
        .bind(intent)
        .bind(sentiment)
        .bind(sentiment_score)
        .bind(ai_suggestion)
        .bind(latency_ms)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}
