# rvoip Voice AI — 提供商全景 + 三条路线架构

## 一、全球 Voice AI 提供商矩阵

### 端到端 Speech-to-Speech (无需分离 ASR/TTS，<500ms)

| 提供商 | 产品 | 延迟 | 中文 | 开源 | 部署 | 备注 |
|--------|------|:----:|:----:|:----:|:----:|------|
| **OpenAI** | [GPT Realtime API](https://openai.com/index/introducing-gpt-realtime/) | <300ms | ✅ | ❌ | 云端 | 业界标杆，WebSocket 双向流 |
| **xAI** | [Grok Voice Agent API](https://x.ai/news/grok-voice-agent-api) | <700ms | ✅ | ❌ | 云端 | 全双工，100+ 语言 |
| **Google** | [Gemini Live API](https://docs.cloud.google.com/vertex-ai/generative-ai/docs/live-api) | <500ms | ✅ | ❌ | 云端 | 支持多模态 (音频+视频) |
| **Amazon** | Nova Sonic | <500ms | ✅ | ❌ | AWS | Bedrock 集成 |
| **Azure** | [Azure OpenAI Realtime](https://learn.microsoft.com/en-us/azure/foundry/openai/how-to/realtime-audio) | <300ms | ✅ | ❌ | Azure | GPT Realtime 的 Azure 版 |
| **Kyutai** | [Moshi](https://github.com/kyutai-labs/moshi) | **~200ms** | ⚠️ | ✅ | 本地 GPU | 法国开源，全双工，L4 GPU |
| **智谱** | [GLM-4-Voice](https://github.com/THUDM/GLM-4-Voice) | ~500ms | ✅✅ | ✅ | 本地 GPU | 中英双语，情绪/语速控制 |

### 中国提供商 (云端 API)

| 提供商 | ASR | TTS | LLM | 端到端 | 延迟 | 备注 |
|--------|:---:|:---:|:---:|:------:|:----:|------|
| **阿里云** | [Paraformer/Qwen3-ASR](https://help.aliyun.com/zh/model-studio/qwen-real-time-speech-recognition) | [CosyVoice/Qwen3-TTS](https://help.aliyun.com/zh/model-studio/cosyvoice-websocket-api) | 通义千问 | 级联 | ~500ms | 百炼平台，WebSocket |
| **字节/火山** | 豆包 ASR | Seed-TTS | 豆包 | ✅端到端 | ~400ms | 实时语音大模型，情绪承接 |
| **百度** | DeepSpeech | PaddleSpeech | 文心一言 | 级联 | ~600ms | 端侧+云端 |
| **科大讯飞** | 讯飞 ASR | 讯飞 TTS | 星火 | 级联 | ~400ms | 语音同传大模型 |
| **腾讯云** | ASR | TTS | 混元 | 级联 | ~500ms | RTC+CDN+大模型 |
| **阶跃星辰** | [Step Audio](https://platform.stepfun.com/docs/llm/audio) | Step TTS | Step LLM | 级联 | ~500ms | 语音理解+生成 |

### 国际组件提供商 (可组合)

| 提供商 | 类型 | 延迟 | 特点 |
|--------|------|:----:|------|
| **[Deepgram](https://deepgram.com)** | ASR | ~150ms | 企业级，高准确率 |
| **[ElevenLabs](https://elevenlabs.io)** | TTS | ~75ms | 最佳音质，声音克隆 |
| **[Cartesia](https://cartesia.ai)** | TTS | **~40ms** | 最低延迟 TTS |
| **[Hume AI](https://hume.ai)** | 情绪分析 | 实时 | 语音情绪识别专家 |
| **[PlayAI](https://play.ai)** | TTS | ~100ms | 实时语音智能 |
| **[Rime](https://rime.ai)** | TTS | ~80ms | 客服场景优化 |

### 开源可自部署

| 项目 | 类型 | 延迟 | 中文 | 硬件需求 |
|------|------|:----:|:----:|:--------:|
| [Moshi](https://github.com/kyutai-labs/moshi) | 端到端 S2S | ~200ms | ⚠️ | L4 GPU |
| [GLM-4-Voice](https://github.com/THUDM/GLM-4-Voice) | 端到端 S2S | ~500ms | ✅✅ | A100 GPU |
| [Whisper](https://github.com/openai/whisper) | ASR | ~300ms | ✅ | CPU/GPU |
| [Piper](https://github.com/rhasspy/piper) | TTS | ~75ms | ✅ | CPU |
| [CosyVoice2](https://github.com/FunAudioLLM/CosyVoice) | TTS | ~150ms | ✅✅ | GPU |
| [Qwen3-TTS](https://github.com/QwenLM/Qwen-TTS) | TTS | ~200ms | ✅✅ | GPU |
| [Kyutai Pocket TTS](https://kyutai.org/tts) | TTS | 实时 | ⚠️ | **CPU 即可** |

---

## 二、三条路线架构

```
┌──────────────────────────────────────────────────────────────────────┐
│                    rvoip AI Copilot                                    │
│                                                                      │
│  ┌────────────────────────────────────────────────────────────────┐  │
│  │                  VoiceAI 统一接口                                │  │
│  │                                                                │  │
│  │  trait VoiceAiProvider {                                        │  │
│  │    /// 端到端: 音频进 → 音频出 + 文本 + 分析                     │  │
│  │    async fn process_audio_stream(                               │  │
│  │      &self,                                                     │  │
│  │      audio_in: Stream<AudioFrame>,                              │  │
│  │      context: &CallContext,                                     │  │
│  │    ) -> Stream<VoiceAiEvent>;                                   │  │
│  │                                                                │  │
│  │    /// 纯文本分析 (已有文本时)                                   │  │
│  │    async fn analyze_text(                                       │  │
│  │      &self,                                                     │  │
│  │      text: &str,                                                │  │
│  │      context: &CallContext,                                     │  │
│  │    ) -> Stream<VoiceAiEvent>;                                   │  │
│  │  }                                                              │  │
│  │                                                                │  │
│  │  enum VoiceAiEvent {                                            │  │
│  │    Transcription { speaker, text, is_final },                   │  │
│  │    Intent { intent, confidence },                               │  │
│  │    Sentiment { value, label },                                  │  │
│  │    Suggestion { text },                                         │  │
│  │    AudioResponse { pcm_data },  // TTS 输出                    │  │
│  │    QualityCheck { checklist },                                  │  │
│  │    KnowledgeRef { articles },                                   │  │
│  │    Error { message },                                           │  │
│  │  }                                                              │  │
│  └────────────┬───────────────┬───────────────┬───────────────────┘  │
│               │               │               │                      │
│     ┌─────────▼──────┐ ┌─────▼──────┐ ┌──────▼─────────┐           │
│     │  Route A       │ │ Route B    │ │ Route C         │           │
│     │  Cloud S2S     │ │ Local S2S  │ │ Cascaded        │           │
│     │  (端到端云端)   │ │ (端到端本地)│ │ (级联管道)       │           │
│     └─────────┬──────┘ └─────┬──────┘ └──────┬─────────┘           │
│               │               │               │                      │
└───────────────┼───────────────┼───────────────┼──────────────────────┘
                │               │               │
                ▼               ▼               ▼
```

### Route A: Cloud Speech-to-Speech (端到端云端, <300ms)

```
客户 RTP 音频
    │
    │ AudioTap (PCM 16kHz)
    ▼
┌────────────────────────────────┐
│  Cloud S2S Provider            │
│                                │
│  配置选择:                     │
│  ├─ openai_realtime            │  GPT Realtime API (WebSocket)
│  ├─ grok_voice                 │  Grok Voice Agent API (WebSocket)
│  ├─ gemini_live                │  Gemini Live API (WebSocket)
│  ├─ azure_realtime             │  Azure OpenAI Realtime
│  ├─ doubao_realtime            │  豆包实时语音 (字节)
│  └─ aliyun_bailian             │  阿里百炼 (Qwen3 级联但延迟低)
│                                │
│  WebSocket 双向流:             │
│  → 发送: PCM 音频帧            │
│  ← 接收: 转写 + AI 回复音频    │
│  ← 接收: function_call 结果    │
│                                │
│  System Instructions:          │
│  (包含知识库 RAG + 话术 +      │
│   质检规则 + 客户信息)          │
│                                │
│  Function Calling / Tools:     │
│  - search_knowledge(query)     │
│  - get_talk_script(scenario)   │
│  - check_quality(checklist)    │
│  - get_customer_info(number)   │
└───────────┬────────────────────┘
            │
            ▼
    VoiceAiEvent Stream
    ├─ Transcription (实时转写)
    ├─ AudioResponse (AI 语音回复)
    ├─ Intent / Sentiment
    └─ Suggestion / Quality
            │
    ┌───────┼───────────┐
    │       │           │
    ▼       ▼           ▼
 WebSocket  DB 异步     RTP 播放
 → 前端     记录        → 客户听到
```

**延迟: ~200-300ms** (取决于提供商)
**优势:** 最低延迟，最高质量，支持 Function Calling
**劣势:** 成本高，依赖外部服务

---

### Route B: Local Speech-to-Speech (端到端本地, ~200-500ms)

```
客户 RTP 音频
    │
    │ AudioTap (PCM 16kHz)
    ▼
┌────────────────────────────────┐
│  Local S2S Model               │
│                                │
│  配置选择:                     │
│  ├─ moshi                      │  Moshi (Kyutai, ~200ms, L4 GPU)
│  ├─ glm4_voice                 │  GLM-4-Voice (智谱, 中文优秀, A100)
│  └─ custom                     │  任何 S2S 模型
│                                │
│  模型加载:                     │
│  - GPU 推理 (CUDA/Metal)       │
│  - 模型文件本地存放             │
│  - 启动时预加载到 GPU          │
│                                │
│  输入: PCM 音频帧 (连续流)     │
│  输出:                         │
│  - 文本转写 (inner monologue)  │
│  - AI 回复音频 (PCM)           │
│  - 可选: 文本分析由独立 LLM 做 │
└───────────┬────────────────────┘
            │
            ▼
    VoiceAiEvent Stream
            │
    ┌───────┼───────────┐
    │       │           │
    ▼       ▼           ▼
 WebSocket  DB 异步     RTP 播放
 → 前端     记录        → 客户听到
            │
            ▼ (可选: 发送转写文本给 PRX/LLM 做深度分析)
    ┌────────────────────┐
    │  PRX / LLM         │
    │  (异步, 不阻塞主管道)│
    │  → 意图识别         │
    │  → RAG 知识库       │
    │  → 质检评分         │
    └────────────────────┘
```

**延迟: ~200ms (Moshi) / ~500ms (GLM-4-Voice)**
**优势:** 完全私有，零 API 成本，可离线运行
**劣势:** 需要 GPU 服务器，模型维护

---

### Route C: Cascaded Pipeline (级联管道, ~800-1500ms)

```
客户 RTP 音频
    │
    │ AudioTap (PCM 16kHz)
    ▼
┌─ Stage 1: ASR ─────────────────┐
│                                 │
│  配置选择:                      │
│  ├─ whisper_local               │  Whisper (本地, ~300ms)
│  ├─ whisper_api                 │  OpenAI Whisper API (~200ms)
│  ├─ deepgram                    │  Deepgram (~150ms)
│  ├─ aliyun_asr                  │  阿里 Paraformer (~200ms)
│  ├─ iflytek_asr                 │  讯飞 ASR (~200ms)
│  └─ azure_speech                │  Azure Speech (~200ms)
│                                 │
│  输出: 文本 + 置信度 + 时间戳   │
└───────────┬─────────────────────┘
            │
            ▼
┌─ Stage 2: LLM 分析 ────────────┐
│                                 │
│  通过 PRX 或直连:               │
│  ├─ prx → (Claude/GPT/Gemini)  │
│  ├─ openai_compatible           │
│  ├─ anthropic                   │
│  ├─ ollama (本地)               │
│  ├─ aliyun_qwen                 │  通义千问
│  ├─ doubao                      │  豆包
│  └─ deepseek                    │  DeepSeek
│                                 │
│  System Prompt 注入:            │
│  - 知识库 RAG                   │
│  - 话术模板                     │
│  - 质检规则                     │
│  - 客户/坐席信息                │
│                                 │
│  流式输出:                      │
│  - intent, sentiment            │
│  - suggestion text              │
│  - quality checklist            │
└───────────┬─────────────────────┘
            │
            ▼
┌─ Stage 3: TTS (可选) ──────────┐
│                                 │
│  配置选择:                      │
│  ├─ piper_local                 │  Piper (本地 CPU, ~75ms)
│  ├─ cosyvoice                   │  CosyVoice (阿里, ~150ms)
│  ├─ elevenlabs                  │  ElevenLabs (~75ms)
│  ├─ cartesia                    │  Cartesia (~40ms)
│  ├─ edge_tts                    │  微软 Edge TTS (免费)
│  ├─ kyutai_pocket               │  Kyutai Pocket TTS (CPU 实时)
│  └─ disabled                    │  不启用 TTS
│                                 │
│  输出: PCM 音频流               │
└───────────┬─────────────────────┘
            │
            ▼
    VoiceAiEvent Stream
            │
    ┌───────┼───────────┐
    │       │           │
    ▼       ▼           ▼
 WebSocket  DB 异步     RTP 播放
 → 前端     记录        → 客户听到
```

**延迟: ~800-1500ms** (ASR + LLM + TTS 串行)
**优势:** 最灵活，每个组件可独立替换，成本可控
**劣势:** 延迟最高

---

## 三、配置系统

```toml
[voice_ai]
# 路线选择: "cloud_s2s" | "local_s2s" | "cascaded"
route = "cloud_s2s"

# ─── Route A: Cloud Speech-to-Speech ───
[voice_ai.cloud_s2s]
# 提供商: openai_realtime | grok_voice | gemini_live | azure_realtime | doubao_realtime
provider = "openai_realtime"

[voice_ai.cloud_s2s.openai_realtime]
api_key = "sk-xxx"
model = "gpt-4o-realtime-preview"
voice = "alloy"

[voice_ai.cloud_s2s.grok_voice]
api_key = "xai-xxx"
model = "grok-3-fast"

[voice_ai.cloud_s2s.gemini_live]
api_key = "xxx"
model = "gemini-2.5-flash"

[voice_ai.cloud_s2s.doubao_realtime]
api_key = "xxx"
endpoint = "wss://rtc.volcengineapi.com"

[voice_ai.cloud_s2s.aliyun_bailian]
api_key = "xxx"
endpoint = "wss://dashscope.aliyuncs.com"

# ─── Route B: Local Speech-to-Speech ───
[voice_ai.local_s2s]
# 模型: moshi | glm4_voice
model = "moshi"
model_path = "/models/moshi"
device = "cuda:0"               # cuda:0 | cpu | mps (Apple Silicon)

# ─── Route C: Cascaded Pipeline ───
[voice_ai.cascaded]

[voice_ai.cascaded.asr]
# whisper_local | whisper_api | deepgram | aliyun_asr | iflytek_asr | azure_speech
provider = "whisper_local"
model_path = "models/whisper-base.bin"
language = "auto"

[voice_ai.cascaded.llm]
# prx | openai | anthropic | ollama | aliyun_qwen | doubao | deepseek
provider = "prx"
[voice_ai.cascaded.llm.prx]
url = "http://127.0.0.1:8200"
model = "default"
[voice_ai.cascaded.llm.openai]
api_key = "sk-xxx"
model = "gpt-4o"
url = "https://api.openai.com/v1"
[voice_ai.cascaded.llm.ollama]
url = "http://localhost:11434"
model = "qwen2:7b"

[voice_ai.cascaded.tts]
# piper_local | cosyvoice | elevenlabs | cartesia | edge_tts | disabled
provider = "piper_local"
model_path = "models/piper-zh-cn.onnx"
voice = "default"

# ─── 通用配置 ───
[voice_ai.features]
realtime_transcription = true
intent_detection = true
sentiment_analysis = true
response_suggestion = true
realtime_quality = true
auto_summary = true
knowledge_rag = true
conversation_logging = true     # 全上下文异步写入 DB

[voice_ai.rag]
knowledge_search_limit = 5
include_talk_scripts = true
include_quality_rules = true
```

## 四、延迟对比

```
                    0ms    200ms   500ms    1000ms   1500ms
                    │       │       │        │        │
Route A (Cloud S2S) │███████│       │        │        │
  GPT Realtime      │  ~250ms                         │
  Grok Voice        │████████████│                     │
                    │    ~700ms  │                     │
  Gemini Live       │██████████│                       │
                    │   ~500ms │                       │
                    │          │                       │
Route B (Local S2S) │       │  │                       │
  Moshi (L4 GPU)    │████│     │                       │
                    │~200ms    │                       │
  GLM-4-Voice       │██████████│                       │
                    │   ~500ms │                       │
                    │          │                       │
Route C (Cascaded)  │          │                       │
  最优组合          │          │████████████████│      │
  (Deepgram+GPT+    │          │     ~800ms    │      │
   Cartesia)        │          │               │      │
  标准组合          │          │               │███████│
  (Whisper+PRX+     │          │               │~1200ms│
   Piper)           │          │               │      │
                    │          │               │      │
人类反应时间        │          │               │      │
                    │████████████████│                  │
                    │    ~600ms     │                   │
```

## 五、推荐配置

| 场景 | 推荐路线 | 配置 | 延迟 | 月成本/千通话 |
|------|:--------:|------|:----:|:------------:|
| **追求极致体验** | Route A | GPT Realtime | ~250ms | ~$500 |
| **中国市场** | Route A | 豆包实时/阿里百炼 | ~400ms | ~¥300 |
| **隐私优先** | Route B | Moshi + L4 GPU | ~200ms | GPU 成本 |
| **中文隐私** | Route B | GLM-4-Voice + A100 | ~500ms | GPU 成本 |
| **成本敏感** | Route C | Whisper+Ollama+Piper | ~1200ms | ~$0 |
| **均衡** | Route C | Deepgram+PRX+Cartesia | ~800ms | ~$100 |
