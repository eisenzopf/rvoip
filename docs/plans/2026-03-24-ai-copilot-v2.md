# rvoip AI Copilot — 最终方案 v2

## 核心理念

rvoip AI Copilot 只关心一件事：**接收一路 LLM 文本流**。不管背后是 PRX、Claude、GPT、Ollama 还是本地模型，rvoip 都只消费一个统一的 Stream<String>。

```
rvoip AI Copilot 视角:

                    "我不关心你是谁"
                          │
           ┌──────────────┼──────────────┐
           │              │              │
      ┌────▼────┐   ┌────▼────┐   ┌────▼────┐
      │  PRX    │   │ OpenAI  │   │ Ollama  │
      │ Gateway │   │ Direct  │   │ Local   │
      │ (推荐)  │   │         │   │         │
      └────┬────┘   └────┬────┘   └────┬────┘
           │              │              │
           └──────────────┼──────────────┘
                          │
                 Stream<String> (统一接口)
                          │
                          ▼
                 rvoip AI Copilot
```

## 一、LLM 接口设计

```rust
/// rvoip 的 LLM 抽象 — 任何提供商都实现这个
#[async_trait]
pub trait LlmStream: Send + Sync {
    /// 流式对话 — 返回一个文本片段的异步 Stream
    async fn chat_stream(
        &self,
        messages: &[Message],
        options: &LlmOptions,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<String>> + Send>>>;

    /// 非流式对话 (便捷方法)
    async fn chat(&self, messages: &[Message], options: &LlmOptions) -> Result<String> {
        let mut stream = self.chat_stream(messages, options).await?;
        let mut result = String::new();
        while let Some(chunk) = stream.next().await {
            result.push_str(&chunk?);
        }
        Ok(result)
    }

    /// 提供商名称
    fn name(&self) -> &str;
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,        // System / User / Assistant
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct LlmOptions {
    pub model: Option<String>,    // 可选覆盖模型
    pub temperature: f64,
    pub max_tokens: Option<u32>,
    pub stop_sequences: Vec<String>,
}
```

## 二、多提供商实现

### Provider 1: PRX (推荐 — 最灵活)

PRX 作为主脑时，rvoip 只需要连接 PRX 的 HTTP API：

```rust
pub struct PrxProvider {
    base_url: String,        // PRX 服务地址
    api_key: Option<String>, // PRX 认证
    model: String,           // PRX 路由的模型名
    client: reqwest::Client,
}

// PRX 暴露 OpenAI 兼容 API
// POST {base_url}/v1/chat/completions  stream=true
// PRX 内部决定路由到 Claude/GPT/Gemini/Ollama
```

优势：
- PRX 管理所有 LLM 密钥和路由
- rvoip 零配置 LLM
- PRX 可做负载均衡、容错、成本控制
- 换模型只改 PRX 配置，rvoip 不动

### Provider 2: OpenAI 兼容 (直连)

```rust
pub struct OpenAiCompatibleProvider {
    base_url: String,        // https://api.openai.com/v1 或任何兼容 API
    api_key: String,
    model: String,
    client: reqwest::Client,
}

// 标准 OpenAI Chat Completions API with stream=true
// 兼容: OpenAI, Azure, DeepSeek, Groq, Together, vLLM, LM Studio
```

### Provider 3: Anthropic 直连

```rust
pub struct AnthropicProvider {
    api_key: String,
    model: String,           // claude-sonnet-4-20250514
    client: reqwest::Client,
}

// Anthropic Messages API with stream=true
// SSE: event: content_block_delta
```

### Provider 4: Ollama 本地

```rust
pub struct OllamaProvider {
    base_url: String,        // http://localhost:11434
    model: String,           // llama3, qwen2, etc.
    client: reqwest::Client,
}

// Ollama API: POST /api/chat  stream=true
// 纯本地，零成本，隐私安全
```

## 三、配置

```toml
[ai]
enabled = true

# LLM 提供商选择
[ai.llm]
# 可选: "prx" | "openai" | "anthropic" | "ollama"
provider = "prx"

# PRX 配置 (当 provider = "prx")
[ai.llm.prx]
url = "http://127.0.0.1:8200"     # PRX 服务地址
model = "default"                   # PRX 内部路由
api_key = ""                        # 可选

# OpenAI 兼容配置 (当 provider = "openai")
[ai.llm.openai]
url = "https://api.openai.com/v1"  # 或任何兼容 API
api_key = "sk-xxx"
model = "gpt-4o"

# Anthropic 配置 (当 provider = "anthropic")
[ai.llm.anthropic]
api_key = "sk-ant-xxx"
model = "claude-sonnet-4-20250514"

# Ollama 配置 (当 provider = "ollama")
[ai.llm.ollama]
url = "http://localhost:11434"
model = "qwen2:7b"

# ASR 配置
[ai.asr]
engine = "whisper-local"            # whisper-local | whisper-api
model_path = "models/whisper-base.bin"
language = "auto"                   # zh | en | auto

# TTS 配置
[ai.tts]
engine = "piper-local"             # piper-local | edge-tts | disabled
model_path = "models/piper-zh-cn.onnx"
voice = "default"

# 功能开关
[ai.features]
realtime_transcription = true       # 实时转写
intent_detection = true             # 意图识别
sentiment_analysis = true           # 情绪分析
response_suggestion = true          # 话术建议
realtime_quality = true             # 实时质检
auto_summary = true                 # 通话后自动摘要
knowledge_rag = true                # 知识库 RAG
conversation_logging = true         # 全上下文异步记录
```

## 四、完整数据流

```
┌─── 一通电话的完整生命周期 ────────────────────────────────────────┐
│                                                                    │
│  1. 来电                                                          │
│     │                                                             │
│  2. IVR / 队列 / 坐席接听                                         │
│     │                                                             │
│  3. 通话中 (实时管道启动)                                          │
│     │                                                             │
│     ├─ RTP Audio Tap ──▶ Whisper ASR ──▶ 客户文本                 │
│     │                         │                                    │
│     │                    异步写入 DB ◀─── conversation_turns       │
│     │                         │           (speaker=customer)       │
│     │                         ▼                                    │
│     │                 ┌─ LLM Stream (PRX/OpenAI/...) ─┐           │
│     │                 │                                │           │
│     │                 │  System Prompt:                 │           │
│     │                 │  "你是呼叫中心AI助手..."        │           │
│     │                 │                                │           │
│     │                 │  Context:                      │           │
│     │                 │  - 客户刚说: "网络连不上"       │           │
│     │                 │  - 历史对话: [turn0, turn1...]  │           │
│     │                 │  - 客户情绪: 焦虑↑             │           │
│     │                 │                                │           │
│     │                 │  Knowledge (RAG):              │           │
│     │                 │  - 网络故障排除指南 [相关度0.92]│           │
│     │                 │  - 路由器重置步骤 [相关度0.85] │           │
│     │                 │                                │           │
│     │                 │  Talk Script:                  │           │
│     │                 │  - 客户投诉标准话术            │           │
│     │                 │                                │           │
│     │                 │  Quality Rules:                │           │
│     │                 │  - ☐ 已确认客户身份            │           │
│     │                 │  - ☑ 使用专业用语              │           │
│     │                 │  - ☐ 提供解决方案              │           │
│     │                 └────────────┬───────────────────┘           │
│     │                              │                               │
│     │                    ┌─────────▼─────────┐                    │
│     │                    │ LLM 流式输出:      │                    │
│     │                    │ • intent: 网络故障  │                    │
│     │                    │ • sentiment: -0.3   │                    │
│     │                    │ • suggestion: "..."  │                    │
│     │                    │ • quality: [...]     │                    │
│     │                    └─────────┬─────────┘                    │
│     │                              │                               │
│     │                    异步写入 DB ◀── conversation_turns        │
│     │                              │     (speaker=ai)              │
│     │                              │                               │
│     │              ┌───────────────┼───────────────┐              │
│     │              │               │               │              │
│     │         WebSocket        Piper TTS      质检引擎            │
│     │         → 前端面板       → RTP 播放     → 实时评分          │
│     │         (坐席看到)       (客户听到,     (异步记录)          │
│     │                          可选)                              │
│     │                                                             │
│  4. 挂断                                                          │
│     │                                                             │
│  5. 后处理 (异步)                                                  │
│     │                                                             │
│     └─▶ LLM 生成通话摘要 ──▶ call_ai_summaries 表                │
│         • 摘要文本                                                 │
│         • 关键话题                                                 │
│         • 情绪变化曲线 [中性 → 焦虑 → 缓和 → 满意]               │
│         • AI 质检评分 85/100                                      │
│         • 改进建议                                                 │
│         • 客户满意度预估                                           │
│                                                                    │
└────────────────────────────────────────────────────────────────────┘
```

## 五、数据库 Schema

```sql
-- 通话逐轮记录
CREATE TABLE conversation_turns (
    id BIGSERIAL PRIMARY KEY,
    call_id TEXT NOT NULL,
    turn_index INTEGER NOT NULL,
    speaker TEXT NOT NULL,           -- customer | agent | ai | system
    asr_text TEXT,                   -- ASR 转写原文
    asr_confidence REAL,
    intent TEXT,
    sentiment TEXT,
    sentiment_score REAL,
    ai_suggestion TEXT,
    tts_text TEXT,                   -- TTS 合成的文本
    knowledge_refs TEXT,             -- 引用的知识库 ID
    latency_ms INTEGER,
    audio_duration_ms INTEGER,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE(call_id, turn_index)
);

-- 通话 AI 摘要
CREATE TABLE call_ai_summaries (
    id BIGSERIAL PRIMARY KEY,
    call_id TEXT NOT NULL UNIQUE,
    summary TEXT,
    key_topics TEXT,                  -- JSON array
    sentiment_arc TEXT,               -- JSON: [{t:0,v:0.1},{t:30,v:-0.5}...]
    overall_sentiment TEXT,
    quality_score INTEGER,
    quality_details TEXT,             -- JSON
    improvement_suggestions TEXT,
    customer_satisfaction TEXT,
    resolution_status TEXT,
    model_used TEXT,
    processing_time_ms INTEGER,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- AI 配置 (runtime 可改)
CREATE TABLE ai_config (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    description TEXT,
    updated_at TIMESTAMPTZ DEFAULT NOW()
);
```

## 六、Crate 结构

```
rvoip/crates/ai-copilot/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── config.rs                # AI 配置 (TOML + DB)
│   │
│   ├── llm/                     # LLM 抽象层
│   │   ├── mod.rs               # LlmStream trait 定义
│   │   ├── prx.rs               # PRX 实现 (OpenAI 兼容 API)
│   │   ├── openai.rs            # OpenAI/兼容 API 直连
│   │   ├── anthropic.rs         # Anthropic Messages API
│   │   ├── ollama.rs            # Ollama 本地
│   │   └── factory.rs           # 根据配置创建 provider
│   │
│   ├── asr/                     # ASR 层
│   │   ├── mod.rs               # AsrEngine trait
│   │   ├── whisper.rs           # whisper-rs 本地
│   │   └── buffer.rs            # 音频缓冲 + VAD
│   │
│   ├── tts/                     # TTS 层
│   │   ├── mod.rs               # TtsEngine trait
│   │   └── piper.rs             # piper-rs 本地
│   │
│   ├── pipeline/                # 核心管道
│   │   ├── mod.rs               # CopilotPipeline 主编排
│   │   ├── audio_tap.rs         # media-core 音频截取
│   │   ├── context.rs           # CallContext 通话上下文
│   │   ├── rag.rs               # 知识库 RAG 检索
│   │   ├── prompts.rs           # System Prompt 模板
│   │   └── recorder.rs          # 异步 DB 写入
│   │
│   ├── analysis/                # 后处理
│   │   ├── summary.rs           # 通话摘要
│   │   └── quality.rs           # AI 质检
│   │
│   └── api/                     # 对外接口
│       ├── rest.rs              # REST: /ai/*
│       ├── ws_copilot.rs        # WebSocket: /ws/copilot/:agent_id
│       └── events.rs            # 事件发布
│
└── models/                      # 本地模型 (gitignore)
    ├── whisper-base.bin
    └── piper-zh-cn.onnx
```

## 七、关键交互

### 前端 Copilot 面板 (WebSocket 推送)

```json
// 服务端 → 前端 (每个事件一条)

// 1. 实时转写
{"type": "transcription", "speaker": "customer", "text": "我的网络连不上了", "final": true}

// 2. 意图识别
{"type": "intent", "intent": "network_troubleshoot", "confidence": 0.95}

// 3. 情绪变化
{"type": "sentiment", "value": -0.3, "label": "frustrated"}

// 4. AI 建议话术 (流式)
{"type": "suggestion_start"}
{"type": "suggestion_chunk", "text": "我理解"}
{"type": "suggestion_chunk", "text": "您的困扰，"}
{"type": "suggestion_chunk", "text": "让我帮您排查..."}
{"type": "suggestion_end"}

// 5. 知识库推荐
{"type": "knowledge", "articles": [{"id": "ART-001", "title": "网络故障排除", "relevance": 0.92}]}

// 6. 质检提醒
{"type": "quality", "checklist": [
  {"item": "确认客户身份", "checked": false, "reminder": true},
  {"item": "使用专业用语", "checked": true}
]}
```

### 系统与 AI 的关系

```
话术来自系统 ──▶ talk_scripts 表
知识来自系统 ──▶ knowledge_articles 表
质检规则来自系统 ──▶ qc_templates + qc_template_items 表
坐席信息来自系统 ──▶ agents 表
客户信息来自系统 ──▶ phone_lists 表 (VIP 识别)

AI 只是消费这些数据，通过 System Prompt 注入给 LLM:
┌──────────────────────────────────────────────┐
│ System Prompt (动态拼装):                     │
│                                              │
│ 你是 rvoip 呼叫中心的 AI 助手。              │
│                                              │
│ 当前通话信息:                                │
│ - 坐席: 李伟 (AGT-001), 技术支持部           │
│ - 客户: 138xxxx001 (VIP 三星)                │
│ - 队列: support                              │
│                                              │
│ 相关知识库 (根据对话检索):                    │
│ [1] 网络故障排除指南: 第一步检查路由器...     │
│ [2] 常见宽带问题: 确认光猫指示灯...          │
│                                              │
│ 参考话术:                                    │
│ [投诉场景] "尊敬的客户，非常抱歉..."         │
│                                              │
│ 质检要求:                                    │
│ - 必须确认客户身份                           │
│ - 必须提供解决方案或转接                     │
│ - 必须使用敬语                               │
│                                              │
│ 你的任务:                                    │
│ 1. 分析客户意图                              │
│ 2. 判断客户情绪 (-1到1)                      │
│ 3. 推荐回复话术                              │
│ 4. 检查质检项目完成情况                      │
│                                              │
│ 以 JSON 格式返回。                           │
└──────────────────────────────────────────────┘
```

## 八、实施步骤

| Step | 内容 | 天数 | 依赖 |
|:----:|------|:----:|:----:|
| 1 | LLM 抽象层 + 4 个 Provider 实现 | 2 | 无 |
| 2 | DB Schema + 异步记录器 + REST API (/ai/*) | 2 | Step 1 |
| 3 | 前端 Copilot 面板 + WebSocket | 2 | Step 2 |
| 4 | ASR 集成 (whisper-rs + audio tap) | 3 | Step 1 |
| 5 | TTS 集成 (piper-rs + RTP 播放) | 2 | Step 4 |
| 6 | RAG + 后处理 (摘要/质检) | 2 | Step 2 |
| **总计** | | **13 天** | |
