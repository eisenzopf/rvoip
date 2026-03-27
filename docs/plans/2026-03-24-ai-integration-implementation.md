# rvoip AI 集成实施计划 — MCP Server + CLI + AI Copilot

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 为 rvoip 呼叫中心平台添加 AI-Native 能力层：MCP Server（让 AI 模型直接操作系统）、CLI 命令行工具、AI Copilot（通话实时辅助）。

**Architecture:** 三个新 crate 并行开发，共享 web-console 的 HTTP API 作为后端。MCP Server 通过 stdio/SSE 暴露 Tools，CLI 通过 HTTP 调用 API，AI Copilot 通过 WebSocket 接收通话事件并调用 LLM。

**Tech Stack:** Rust 2024 / rmcp (Rust MCP SDK) / clap 4 / Claude API (Anthropic SDK) / Whisper/实时 ASR

---

## Phase A: MCP Server (最高优先级)

### A.1 Crate 结构

```
rvoip/crates/mcp-server/
├── Cargo.toml
├── src/
│   ├── lib.rs              # 模块定义 + RvoipMcpServer 公共 API
│   ├── main.rs             # 入口: stdio 或 SSE 模式
│   ├── server.rs           # MCP Server 实现 (impl ServerHandler)
│   ├── api_client.rs       # HTTP 客户端 (调用 web-console API)
│   ├── tools/
│   │   ├── mod.rs          # Tool 注册表
│   │   ├── calls.rs        # 通话类 Tools (6个)
│   │   ├── agents.rs       # 坐席类 Tools (6个)
│   │   ├── queues.rs       # 队列类 Tools (5个)
│   │   ├── routing.rs      # 路由类 Tools (3个)
│   │   ├── knowledge.rs    # 知识库类 Tools (4个)
│   │   ├── system.rs       # 系统类 Tools (4个)
│   │   ├── users.rs        # 用户类 Tools (4个)
│   │   ├── departments.rs  # 部门类 Tools (3个)
│   │   └── reports.rs      # 报表类 Tools (3个)
│   ├── resources/
│   │   └── mod.rs          # Resource 提供者 (5个)
│   └── prompts/
│       └── mod.rs          # 预置 Prompt 模板 (3个)
└── README.md
```

### A.2 依赖

```toml
[package]
name = "rvoip-mcp-server"
version.workspace = true
edition.workspace = true

[dependencies]
rmcp = { version = "0.1", features = ["server", "transport-sse", "transport-io"] }
tokio = { workspace = true, features = ["full"] }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
reqwest = { version = "0.12", features = ["json"] }
clap = { workspace = true, features = ["derive"] }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
anyhow = { workspace = true }
```

### A.3 Tool 清单 (38 个 Tools)

#### 通话管理 (6)
| Tool | 描述 | 参数 | 返回 |
|------|------|------|------|
| `list_active_calls` | 列出活跃通话 | status? | 通话列表 |
| `get_call_detail` | 获取通话详情 | call_id | 通话详情 |
| `hangup_call` | 挂断通话 | call_id | 确认 |
| `get_call_history` | 查询历史通话 | limit?, offset?, agent_id? | 历史列表 |
| `get_call_stats` | 通话统计 | period? (today/week/month) | 统计数据 |
| `transfer_call` | 转接通话 | call_id, target_agent_id | 确认 |

#### 坐席管理 (6)
| Tool | 描述 | 参数 | 返回 |
|------|------|------|------|
| `list_agents` | 列出坐席 | status?, department? | 坐席列表 |
| `create_agent` | 创建坐席 | display_name, department?, skills?, max_calls? | 新坐席 |
| `update_agent` | 更新坐席 | agent_id, fields... | 确认 |
| `delete_agent` | 删除坐席 | agent_id | 确认 |
| `set_agent_status` | 设置状态 | agent_id, status | 确认 |
| `get_agent_performance` | 坐席绩效 | agent_id, period? | 绩效数据 |

#### 队列管理 (5)
| Tool | 描述 | 参数 | 返回 |
|------|------|------|------|
| `list_queues` | 列出队列 | - | 队列列表+状态 |
| `create_queue` | 创建队列 | queue_id | 确认 |
| `get_queue_status` | 队列状态 | queue_id | 详细状态 |
| `assign_call_to_agent` | 手动分配 | queue_id, call_id, agent_id | 确认 |
| `get_queue_performance` | 队列绩效 | period? | 绩效数据 |

#### 路由配置 (3)
| Tool | 描述 | 参数 | 返回 |
|------|------|------|------|
| `get_routing_config` | 获取路由配置 | - | 配置 JSON |
| `list_overflow_policies` | 溢出策略列表 | - | 策略列表 |
| `create_overflow_policy` | 创建溢出策略 | name, condition, action, priority | 新策略 |

#### 知识库 (4)
| Tool | 描述 | 参数 | 返回 |
|------|------|------|------|
| `search_knowledge` | 搜索知识库 | query, category? | 匹配文章 |
| `get_article` | 获取文章 | article_id | 文章内容 |
| `list_talk_scripts` | 话术列表 | category? | 话术列表 |
| `suggest_response` | AI 推荐话术 | context, customer_intent | 推荐回复 |

#### 系统管理 (4)
| Tool | 描述 | 参数 | 返回 |
|------|------|------|------|
| `get_system_health` | 系统健康 | - | 健康状态 |
| `get_dashboard` | 仪表盘数据 | - | KPI 指标 |
| `get_audit_log` | 审计日志 | limit?, offset? | 日志列表 |
| `export_config` | 导出配置 | - | 配置 JSON |

#### 用户管理 (4)
| Tool | 描述 | 参数 | 返回 |
|------|------|------|------|
| `list_users` | 列出用户 | role?, search? | 用户列表 |
| `create_user` | 创建用户 | username, password, roles | 新用户 |
| `update_user_roles` | 分配角色 | user_id, roles | 确认 |
| `delete_user` | 删除用户 | user_id | 确认 |

#### 部门管理 (3)
| Tool | 描述 | 参数 | 返回 |
|------|------|------|------|
| `list_departments` | 列出部门 | - | 部门列表 |
| `create_department` | 创建部门 | name, description?, parent_id? | 新部门 |
| `delete_department` | 删除部门 | department_id | 确认 |

#### 报表 (3)
| Tool | 描述 | 参数 | 返回 |
|------|------|------|------|
| `generate_daily_report` | 生成日报 | date | 日报数据 |
| `generate_agent_report` | 坐席绩效 | start, end, agent_id? | 绩效数据 |
| `generate_summary_report` | 综合报表 | start, end | 综合数据 |

### A.4 Resource 清单 (5 个)

| URI | 描述 | MIME |
|-----|------|------|
| `rvoip://calls/active` | 活跃通话实时列表 | application/json |
| `rvoip://agents/online` | 在线坐席列表 | application/json |
| `rvoip://queues/status` | 队列实时状态 | application/json |
| `rvoip://system/health` | 系统健康状态 | application/json |
| `rvoip://config/current` | 当前系统配置 | application/json |

### A.5 Prompt 模板 (3 个)

| Name | 描述 |
|------|------|
| `call_center_manager` | 呼叫中心管理助手 — 帮助管理员进行日常运营 |
| `quality_reviewer` | 质检评审员 — 分析通话记录并评分 |
| `report_analyst` | 报表分析师 — 生成洞察和建议 |

### A.6 API Client 实现

```rust
// crates/mcp-server/src/api_client.rs

pub struct RvoipApiClient {
    base_url: String,
    token: String,
    client: reqwest::Client,
}

impl RvoipApiClient {
    pub fn new(base_url: &str, token: &str) -> Self { ... }

    // 通用请求方法
    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> { ... }
    async fn post<T: DeserializeOwned>(&self, path: &str, body: &impl Serialize) -> Result<T> { ... }
    async fn put<T: DeserializeOwned>(&self, path: &str, body: &impl Serialize) -> Result<T> { ... }
    async fn delete(&self, path: &str) -> Result<String> { ... }

    // 业务方法 (每个 Tool 对应一个)
    pub async fn list_active_calls(&self) -> Result<Vec<ActiveCall>> { ... }
    pub async fn create_agent(&self, req: CreateAgentRequest) -> Result<AgentView> { ... }
    // ... 38 个方法
}
```

### A.7 使用方式

```bash
# 方式 1: stdio (给 Claude Desktop / Claude Code 用)
rvoip-mcp-server --base-url http://127.0.0.1:3000 --token "jwt_xxx"

# 方式 2: SSE HTTP (给远程 AI 客户端用)
rvoip-mcp-server --mode sse --port 3001 --base-url http://127.0.0.1:3000 --token "jwt_xxx"

# Claude Desktop 配置 (claude_desktop_config.json)
{
  "mcpServers": {
    "rvoip": {
      "command": "rvoip-mcp-server",
      "args": ["--base-url", "http://127.0.0.1:3000", "--token", "jwt_xxx"]
    }
  }
}

# Claude Code 配置 (.mcp.json)
{
  "mcpServers": {
    "rvoip": {
      "command": "rvoip-mcp-server",
      "args": ["--base-url", "http://127.0.0.1:3000", "--token", "jwt_xxx"]
    }
  }
}
```

---

## Phase B: CLI 命令行工具

### B.1 Crate 结构

```
rvoip/crates/cli/
├── Cargo.toml
├── src/
│   ├── main.rs             # clap 入口
│   ├── config.rs           # ~/.rvoip/config.toml 配置管理
│   ├── api_client.rs       # 复用 mcp-server 的 API client
│   ├── output.rs           # 输出格式化 (table/json/csv)
│   └── commands/
│       ├── mod.rs
│       ├── status.rs       # rvoip status
│       ├── agent.rs        # rvoip agent [list|create|update|delete|status]
│       ├── call.rs         # rvoip call [list|history|hangup|transfer]
│       ├── queue.rs        # rvoip queue [list|status|create]
│       ├── user.rs         # rvoip user [list|create|delete|roles]
│       ├── config_cmd.rs   # rvoip config [show|set|export|import]
│       ├── report.rs       # rvoip report [daily|agent|summary|export]
│       ├── department.rs   # rvoip dept [list|create|delete]
│       ├── ivr.rs          # rvoip ivr [list|create|delete]
│       ├── trunk.rs        # rvoip trunk [list|create|delete]
│       ├── mcp.rs          # rvoip mcp serve (启动 MCP Server)
│       └── login.rs        # rvoip login (获取 JWT)
└── README.md
```

### B.2 依赖

```toml
[package]
name = "rvoip-cli"
version.workspace = true
edition.workspace = true

[[bin]]
name = "rvoip"
path = "src/main.rs"

[dependencies]
clap = { workspace = true, features = ["derive", "env"] }
reqwest = { version = "0.12", features = ["json"] }
tokio = { workspace = true, features = ["full"] }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
tabled = "0.17"           # 表格输出
dialoguer = "0.11"        # 交互式输入 (密码输入等)
indicatif = "0.17"        # 进度条
colored = "3"             # 彩色输出
dirs = "6"                # 用户目录 (~/.rvoip/)
toml = "0.8"              # 配置文件
anyhow = { workspace = true }
tracing = { workspace = true }
```

### B.3 命令结构

```
rvoip
├── login                           # 交互式登录，保存 token
├── status                          # 系统状态总览
├── agent
│   ├── list [--status] [--dept]    # 列出坐席
│   ├── create --name --dept        # 创建 (自动分配 ID/分机/SIP)
│   ├── update <id> [--name] ...    # 更新
│   ├── delete <id>                 # 删除
│   ├── status <id> --set <status>  # 改状态
│   └── perf <id> [--period]        # 绩效
├── call
│   ├── list                        # 活跃通话
│   ├── history [--limit] [--agent] # 历史
│   ├── hangup <id>                 # 挂断
│   └── stats [--period]            # 统计
├── queue
│   ├── list                        # 队列列表
│   ├── status <id>                 # 队列状态
│   └── create <id>                 # 创建
├── user
│   ├── list                        # 用户列表
│   ├── create --name --password    # 创建
│   └── roles <id> --set <roles>    # 角色
├── dept
│   ├── list                        # 部门
│   └── create --name               # 创建
├── config
│   ├── show [section]              # 查看配置
│   ├── export                      # 导出 JSON
│   └── import <file>               # 导入
├── report
│   ├── daily [--date]              # 日报
│   ├── agent [--start --end]       # 坐席报表
│   ├── summary [--start --end]     # 综合报表
│   └── export --format csv/json    # 导出
├── mcp
│   └── serve [--mode stdio|sse]    # 启动 MCP Server
└── completion
    └── <shell>                     # Shell 补全脚本生成
```

### B.4 配置文件

```toml
# ~/.rvoip/config.toml
[server]
url = "http://127.0.0.1:3000"

[auth]
token = "eyJ..."       # JWT (rvoip login 后自动写入)
username = "admin"

[output]
format = "table"        # table | json | csv
color = true
```

---

## Phase C: AI Copilot (通话实时辅助)

### C.1 架构

```
通话音频 (RTP)
    │
    ▼
┌──────────────┐
│ ASR Engine   │ ← Whisper / Paraformer / Azure Speech
│ (实时转写)   │
└──────┬───────┘
       │ 文本流
       ▼
┌──────────────┐     ┌──────────────┐
│ NLU Pipeline │────►│ LLM (Claude) │
│ • 意图识别    │     │ • 话术建议    │
│ • 实体提取    │     │ • 知识检索    │
│ • 情绪分析    │     │ • 质检评分    │
└──────┬───────┘     └──────┬───────┘
       │                    │
       ▼                    ▼
┌──────────────────────────────────┐
│ WebSocket → 坐席工作台 (前端)    │
│ • 实时转写字幕                   │
│ • 意图标签                       │
│ • 情绪指示器                     │
│ • AI 建议话术                    │
│ • 相关知识库文章                 │
│ • 实时质检提醒                   │
└──────────────────────────────────┘
```

### C.2 Crate 结构

```
rvoip/crates/ai-copilot/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── asr/                # ASR 引擎抽象
│   │   ├── mod.rs          # ASREngine trait
│   │   ├── whisper.rs      # Whisper 本地/API
│   │   └── azure.rs        # Azure Speech (可选)
│   ├── llm/                # LLM 集成
│   │   ├── mod.rs          # LLMProvider trait
│   │   ├── claude.rs       # Anthropic Claude API
│   │   ├── openai.rs       # OpenAI (可选)
│   │   └── local.rs        # Ollama/本地模型 (可选)
│   ├── pipeline/           # AI 处理管道
│   │   ├── mod.rs
│   │   ├── transcription.rs  # 实时转写
│   │   ├── intent.rs         # 意图识别
│   │   ├── sentiment.rs      # 情绪分析
│   │   ├── suggestion.rs     # 话术建议 (RAG)
│   │   └── quality.rs        # 实时质检
│   ├── api/                # REST + WebSocket API
│   │   ├── mod.rs
│   │   ├── ws_handler.rs   # WebSocket 处理 (坐席连接)
│   │   └── rest.rs         # REST 端点 (摘要、分析)
│   └── config.rs           # AI 配置
└── examples/
    ├── transcribe_demo.rs   # ASR 演示
    └── copilot_demo.rs      # 完整 Copilot 演示
```

### C.3 API 端点

```
# 集成到 web-console 的 API
POST /api/v1/ai/transcribe       # 上传音频 → 转写文本
POST /api/v1/ai/analyze          # 分析通话文本 → 意图+情绪
POST /api/v1/ai/summarize        # 通话文本 → 摘要
POST /api/v1/ai/suggest          # 客户问题 → 推荐话术
POST /api/v1/ai/quality-score    # 通话文本 → 质检评分

# WebSocket (坐席实时辅助)
WS /ws/copilot/:agent_id         # 坐席连接 → 实时推送转写/建议/情绪
```

### C.4 前端 AI Copilot 面板

```
新增前端组件:
├── src/components/ai-copilot.tsx    # AI 面板组件
├── src/hooks/useCopilot.ts          # Copilot WebSocket hook
└── src/pages/Softphone.tsx          # 增强: 集成 Copilot 面板

AI 面板显示:
┌─ AI Copilot ────────────────────┐
│ 📝 实时转写                      │
│ [客户] 我的网络连不上了，已经...  │
│ [坐席] 好的，我帮您检查一下...   │
│                                  │
│ 🧠 意图: 网络故障报修            │
│ 😊 情绪: 中性 → 焦虑 (↑)        │
│                                  │
│ 💡 建议回复:                     │
│ "我理解您的焦虑，让我们一步步    │
│  排查问题。请先检查路由器..."    │
│                    [使用此回复]   │
│                                  │
│ 📚 相关知识:                     │
│ • 网络故障排除指南 [查看]        │
│ • 路由器重置步骤   [查看]        │
│                                  │
│ ✅ 质检: 已确认身份 ✓            │
│         专业用语 ✓               │
│         问题解决 进行中...        │
└──────────────────────────────────┘
```

### C.5 LLM 集成配置

```toml
# AI 配置 (环境变量或配置文件)
[ai]
# LLM 提供商
llm_provider = "claude"             # claude | openai | local
anthropic_api_key = "sk-ant-xxx"    # Claude API Key
model = "claude-sonnet-4-20250514"

# ASR 提供商
asr_provider = "whisper"            # whisper | azure | local
whisper_model = "base"              # tiny | base | small | medium | large

# 功能开关
enable_transcription = true
enable_intent_detection = true
enable_sentiment_analysis = true
enable_suggestions = true
enable_realtime_quality = true

# RAG 配置 (知识库检索)
knowledge_search_limit = 5
suggestion_max_tokens = 200
```

---

## 四、任务拆分与排期

### Phase A: MCP Server (5 天)

| 任务 | 天数 | 说明 |
|------|:----:|------|
| A1: 创建 crate 骨架 + API Client | 0.5 | Cargo.toml, api_client.rs, 认证 |
| A2: 通话管理 Tools (6个) | 0.5 | list, detail, hangup, history, stats, transfer |
| A3: 坐席管理 Tools (6个) | 0.5 | list, create, update, delete, status, performance |
| A4: 队列+路由+部门 Tools (11个) | 0.5 | 队列5 + 路由3 + 部门3 |
| A5: 知识库+系统+用户+报表 (15个) | 1.0 | 知识4 + 系统4 + 用户4 + 报表3 |
| A6: Resources + Prompts | 0.5 | 5个 Resource + 3个 Prompt |
| A7: 测试 + Claude Desktop 集成验证 | 1.0 | 端到端测试 |
| A8: README + 文档 | 0.5 | 使用说明 |

### Phase B: CLI (3 天)

| 任务 | 天数 | 说明 |
|------|:----:|------|
| B1: 创建 crate + config + login | 0.5 | 骨架, 配置文件, 认证 |
| B2: status + agent + call 命令 | 0.5 | 核心命令 |
| B3: queue + user + dept + config 命令 | 0.5 | 管理命令 |
| B4: report + ivr + trunk 命令 | 0.5 | 扩展命令 |
| B5: mcp serve 子命令 (复用 A) | 0.5 | 集成 MCP Server |
| B6: 输出格式化 + shell completion | 0.5 | 表格/JSON/CSV + zsh/bash 补全 |

### Phase C: AI Copilot (10 天)

| 任务 | 天数 | 说明 |
|------|:----:|------|
| C1: ASR 引擎抽象 + Whisper 集成 | 2.0 | ASREngine trait + whisper-rs |
| C2: LLM 集成 (Claude API) | 1.0 | 意图识别 + 话术建议 |
| C3: 情绪分析 pipeline | 0.5 | 基于 LLM 的情绪检测 |
| C4: RAG 知识库检索 | 1.0 | 知识库文章检索 + LLM 生成 |
| C5: 实时质检评分 | 1.0 | 对照模板实时评分 |
| C6: WebSocket API | 1.0 | 坐席连接 + 实时推送 |
| C7: 后端 AI 端点集成 | 1.0 | /api/v1/ai/* 端点 |
| C8: 前端 Copilot 面板 | 1.5 | React 组件 + WebSocket hook |
| C9: 端到端测试 | 1.0 | 完整流程验证 |

---

## 五、开发顺序

```
Week 1:     Phase A (MCP Server)
            ├── Day 1-2: A1-A4 (骨架 + 核心 Tools)
            ├── Day 3-4: A5-A6 (全部 Tools + Resources)
            └── Day 5:   A7-A8 (测试 + 文档)

Week 2:     Phase B (CLI) + Phase C 开始
            ├── Day 1-3: B1-B6 (完整 CLI)
            └── Day 4-5: C1-C2 (ASR + LLM 集成)

Week 3:     Phase C (AI Copilot)
            ├── Day 1-2: C3-C5 (情绪+RAG+质检)
            ├── Day 3-4: C6-C8 (WebSocket+API+前端)
            └── Day 5:   C9 (端到端测试)
```

---

## 六、交付物

完成后，rvoip 将提供：

1. **MCP Server** — 38 个 Tools，5 个 Resources，3 个 Prompts
   - Claude Desktop / Claude Code 直接管理呼叫中心
   - 任何 MCP 兼容 AI 客户端可接入

2. **CLI** — 12 个命令组，50+ 子命令
   - 终端管理所有模块
   - 脚本自动化 + CI/CD 集成
   - 内置 MCP Server 模式

3. **AI Copilot** — 5 大 AI 能力
   - 实时语音转写 (ASR)
   - 意图识别 (NLU)
   - 情绪分析
   - RAG 话术建议
   - 实时质检评分
