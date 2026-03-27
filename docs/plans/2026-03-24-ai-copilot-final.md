# rvoip AI Copilot 最终实施方案

## 一、核心设计原则

1. **LLM 不绑死一家** — 通过 [openprx/prx](https://docs.openprx.dev) 网关接入，支持 Claude/GPT/Gemini/本地模型切换
2. **全通话上下文异步采集** — 每句话的 ASR 文本、AI 分析、TTS 回复全部异步写入 DB
3. **实时流式处理** — ASR 流式转写 + LLM 流式生成 + TTS 流式合成，端到端 <1s
4. **话术和知识库来自系统** — RAG 从 knowledge_articles + talk_scripts 检索
5. **全部本地可部署** — Whisper 本地 + Piper TTS 本地 + Ollama 本地 LLM

## 二、架构图

```
┌─────────────────── 实时语音管道 ───────────────────┐
│                                                     │
│  客户 RTP ──▶ AudioTap ──▶ Whisper ASR (流式)      │
│                                │                    │
│                           客户文本流                │
│                                │                    │
│                    ┌───────────▼────────────┐       │
│                    │   PRX LLM Gateway      │       │
│                    │   ┌─────────────────┐  │       │
│                    │   │ System Prompt:  │  │       │
│                    │   │ • 通话上下文    │  │       │
│                    │   │ • 知识库 RAG    │  │       │
│                    │   │ • 话术模板      │  │       │
│                    │   │ • 质检规则      │  │       │
│                    │   └─────────────────┘  │       │
│                    └───────────┬────────────┘       │
│                                │                    │
│                    ┌───────────▼────────────┐       │
│                    │  Piper TTS (流式合成)   │       │
│                    └───────────┬────────────┘       │
│                                │                    │
│                    RTP 播放 ◀──┘                    │
│                                                     │
└────────────────────────┬────────────────────────────┘
                         │
              ┌──────────▼──────────┐
              │   异步上下文采集      │
              │                      │
              │  conversation_turns  │ ← 每一轮对话
              │  ┌──────────────┐   │
              │  │ call_id      │   │
              │  │ turn_index   │   │    turn 0: 客户说 "..."
              │  │ speaker      │   │    turn 1: AI 分析 + 建议
              │  │ asr_text     │   │    turn 2: 坐席/AI 回复 "..."
              │  │ intent       │   │    turn 3: TTS 合成文本
              │  │ sentiment    │   │    ...
              │  │ ai_suggestion│   │
              │  │ tts_text     │   │
              │  │ confidence   │   │
              │  │ latency_ms   │   │
              │  │ timestamp    │   │
              │  └──────────────┘   │
              │                      │
              │  call_ai_summary    │ ← 通话结束后
              │  ┌──────────────┐   │
              │  │ call_id      │   │
              │  │ summary      │   │
              │  │ key_topics   │   │
              │  │ sentiment_arc│   │    [中性→焦虑→满意]
              │  │ quality_score│   │
              │  │ suggestions  │   │
              │  │ created_at   │   │
              │  └──────────────┘   │
              └─────────────────────┘
```

## 三、数据库 Schema

```sql
-- 通话逐轮记录 (每句话一条记录)
CREATE TABLE conversation_turns (
    id BIGSERIAL PRIMARY KEY,
    call_id TEXT NOT NULL,
    turn_index INTEGER NOT NULL,
    speaker TEXT NOT NULL CHECK (speaker IN ('customer', 'agent', 'ai', 'system')),
    asr_text TEXT,                    -- ASR 转写原文
    asr_confidence REAL,              -- ASR 置信度
    intent TEXT,                      -- 识别的意图
    sentiment TEXT,                   -- 情绪 (positive/neutral/negative/angry)
    sentiment_score REAL,             -- 情绪分值 -1.0 ~ 1.0
    ai_suggestion TEXT,               -- AI 建议的回复
    tts_text TEXT,                    -- 实际 TTS 合成的文本
    knowledge_refs TEXT,              -- 引用的知识库文章 ID (逗号分隔)
    latency_ms INTEGER,              -- 该轮处理延迟
    audio_duration_ms INTEGER,       -- 该轮音频时长
    timestamp TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(call_id, turn_index)
);

CREATE INDEX idx_conv_turns_call ON conversation_turns(call_id);
CREATE INDEX idx_conv_turns_time ON conversation_turns(timestamp);

-- 通话 AI 摘要 (通话结束后异步生成)
CREATE TABLE call_ai_summaries (
    id BIGSERIAL PRIMARY KEY,
    call_id TEXT NOT NULL UNIQUE,
    summary TEXT NOT NULL,             -- 通话摘要
    key_topics TEXT,                   -- 关键话题 (JSON array)
    sentiment_arc TEXT,                -- 情绪变化曲线 (JSON array)
    overall_sentiment TEXT,            -- 总体情绪
    quality_score INTEGER,             -- AI 质检评分 0-100
    quality_details TEXT,              -- 质检明细 (JSON)
    improvement_suggestions TEXT,      -- 改进建议
    customer_satisfaction TEXT,        -- 客户满意度估计
    resolution_status TEXT,            -- 问题解决状态
    tags TEXT,                         -- 自动标签
    model_used TEXT,                   -- 使用的 LLM 模型
    processing_time_ms INTEGER,       -- 处理耗时
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- AI 配置表
CREATE TABLE ai_config (
    id TEXT PRIMARY KEY,
    key TEXT NOT NULL UNIQUE,
    value TEXT NOT NULL,
    description TEXT,
    updated_at TIMESTAMPTZ DEFAULT NOW()
);
```

## 四、Crate 结构

```
rvoip/crates/ai-copilot/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── config.rs               # AI 配置 (从 DB + 环境变量)
│   │
│   ├── asr/                    # 语音识别层
│   │   ├── mod.rs              # AsrEngine trait
│   │   ├── whisper_local.rs    # 本地 Whisper (whisper-rs)
│   │   ├── whisper_api.rs      # OpenAI Whisper API
│   │   └── streaming.rs        # 流式 ASR 缓冲管理
│   │
│   ├── tts/                    # 语音合成层
│   │   ├── mod.rs              # TtsEngine trait
│   │   ├── piper_local.rs      # 本地 Piper TTS (piper-rs)
│   │   ├── edge_tts.rs         # 微软 Edge TTS (免费)
│   │   └── streaming.rs        # 流式 TTS 输出
│   │
│   ├── llm/                    # LLM 层 (通过 PRX)
│   │   ├── mod.rs              # LlmProvider trait
│   │   ├── prx_client.rs       # PRX Gateway HTTP 客户端
│   │   ├── prompts.rs          # System Prompt 模板
│   │   └── rag.rs              # 知识库 RAG 检索
│   │
│   ├── pipeline/               # 实时处理管道
│   │   ├── mod.rs              # Pipeline 编排
│   │   ├── audio_tap.rs        # RTP 音频截取 (media-core callback)
│   │   ├── turn_detector.rs    # 轮次检测 (VAD + 静音检测)
│   │   ├── context.rs          # 通话上下文管理
│   │   └── recorder.rs         # 异步 DB 写入
│   │
│   ├── analysis/               # 分析能力
│   │   ├── mod.rs
│   │   ├── intent.rs           # 意图识别
│   │   ├── sentiment.rs        # 情绪分析
│   │   ├── quality.rs          # 实时质检
│   │   └── summary.rs          # 通话摘要 (后处理)
│   │
│   └── api/                    # 对外接口
│       ├── mod.rs
│       ├── ws_copilot.rs       # WebSocket: 坐席 Copilot 面板
│       ├── rest.rs             # REST: 手动分析、历史查询
│       └── events.rs           # 事件发布 (通话开始/结束/轮次)
│
├── models/                     # 本地模型文件
│   ├── whisper-base.bin        # Whisper base 模型 (~150MB)
│   └── piper-zh-cn.onnx       # Piper 中文 TTS (~50MB)
│
└── examples/
    ├── copilot_demo.rs         # 完整演示
    └── transcribe_file.rs      # 音频文件转写
```

## 五、关键 Trait 设计

```rust
/// ASR 引擎 trait — 可插拔
#[async_trait]
pub trait AsrEngine: Send + Sync {
    /// 流式输入音频数据
    async fn feed_audio(&self, pcm_samples: &[i16], sample_rate: u32) -> Result<()>;
    /// 获取当前识别结果 (partial)
    async fn get_partial(&self) -> Result<Option<String>>;
    /// 获取最终结果 (一句话结束)
    async fn get_final(&self) -> Result<Option<AsrResult>>;
}

pub struct AsrResult {
    pub text: String,
    pub confidence: f32,
    pub language: String,
    pub duration_ms: u32,
}

/// TTS 引擎 trait — 可插拔
#[async_trait]
pub trait TtsEngine: Send + Sync {
    /// 合成语音 (流式输出)
    async fn synthesize(&self, text: &str, voice: &str) -> Result<AudioStream>;
}

/// LLM 提供者 trait — 通过 PRX 网关
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// 流式对话
    async fn chat_stream(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
    ) -> Result<Pin<Box<dyn Stream<Item = Result<String>>>>>;
}

/// 通话上下文 — 累积整通电话的状态
pub struct CallContext {
    pub call_id: String,
    pub agent_id: String,
    pub customer_number: String,
    pub turns: Vec<ConversationTurn>,
    pub current_intent: Option<String>,
    pub sentiment_history: Vec<(f32, DateTime<Utc>)>,
    pub referenced_articles: Vec<String>,
    pub started_at: DateTime<Utc>,
}
```

## 六、PRX 集成方式

```rust
/// 通过 PRX Gateway 调用 LLM
pub struct PrxLlmClient {
    prx_url: String,       // PRX 网关地址
    model: String,         // 模型名 (由 PRX 路由到实际提供商)
    client: reqwest::Client,
}

impl PrxLlmClient {
    pub async fn chat(&self, messages: &[ChatMessage]) -> Result<String> {
        // PRX 暴露 OpenAI 兼容 API
        let resp = self.client.post(&format!("{}/v1/chat/completions", self.prx_url))
            .json(&json!({
                "model": self.model,
                "messages": messages,
                "stream": false,
            }))
            .send().await?;
        // ...
    }
}
```

PRX 配置决定实际路由到哪个 LLM 提供商，rvoip 不关心具体是 Claude 还是 GPT。

## 七、实施分步

### Step 1: 基础设施 (2天)
- 创建 ai-copilot crate 骨架
- conversation_turns + call_ai_summaries 表
- AI config 表 + web console 配置页
- PRX LLM Client (OpenAI 兼容 API)

### Step 2: 文本分析 Copilot (2天)
- REST API: POST /ai/analyze → 意图识别 + 情绪分析
- REST API: POST /ai/suggest → RAG 知识库检索 + 话术建议
- REST API: POST /ai/summarize → 通话摘要
- WebSocket: /ws/copilot/:agent_id → 前端面板

### Step 3: ASR 集成 (3天)
- Whisper-rs 本地模型加载
- media-core `set_audio_frame_callback` 音频截取
- 流式 ASR 管道 (音频帧 → PCM 缓冲 → Whisper → 文本)
- 异步写入 conversation_turns

### Step 4: TTS 集成 (2天)
- Piper-rs 本地 TTS
- LLM 回复 → TTS 合成 → RTP 播放
- 支持中文 + 英文 voice

### Step 5: 前端 Copilot 面板 (2天)
- 实时转写字幕
- 意图/情绪指示器
- AI 建议话术 (一键使用)
- 知识库推荐
- 质检提醒

### Step 6: 后处理 + 完善 (2天)
- 通话结束后自动生成摘要
- 自动质检评分
- 情绪曲线可视化
- conversation_turns 查看页面

## 八、配置

```toml
[ai]
enabled = true

[ai.llm]
provider = "prx"                    # prx | direct
prx_url = "http://127.0.0.1:8080"  # PRX Gateway 地址
model = "claude-sonnet-4-20250514"  # PRX 会路由到实际提供商
fallback_model = "gpt-4o-mini"      # 备用模型

[ai.asr]
engine = "whisper-local"            # whisper-local | whisper-api
model_path = "models/whisper-base.bin"
language = "zh"                     # zh | en | auto
streaming = true

[ai.tts]
engine = "piper-local"             # piper-local | edge-tts
model_path = "models/piper-zh-cn.onnx"
voice = "zh-CN-XiaoxiaoNeural"
speed = 1.0

[ai.features]
transcription = true
intent_detection = true
sentiment_analysis = true
suggestions = true
realtime_quality = true
auto_summary = true

[ai.rag]
knowledge_search_limit = 5
min_similarity = 0.7
```
