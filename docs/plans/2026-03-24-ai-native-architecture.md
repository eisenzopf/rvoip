# rvoip AI-Native 通信平台架构设计

## 一、全景架构

```
┌─────────────────────────────────────────────────────────────────────┐
│                        AI Layer                                      │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐ │
│  │ ASR      │ │ TTS      │ │ LLM      │ │ NLU      │ │ Sentiment│ │
│  │ 语音识别  │ │ 语音合成  │ │ 大语言模型│ │ 意图识别  │ │ 情绪分析  │ │
│  │ Whisper  │ │ Coqui/   │ │ Claude/  │ │ Claude   │ │ Claude   │ │
│  │ /Parafor │ │ Edge-TTS │ │ GPT/本地 │ │ Function │ │ Realtime │ │
│  └──────────┘ └──────────┘ └──────────┘ └──────────┘ └──────────┘ │
├─────────────────────────────────────────────────────────────────────┤
│                     MCP Server Layer                                 │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │ rvoip MCP Server                                              │   │
│  │ ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐         │   │
│  │ │ call     │ │ agent    │ │ queue    │ │ knowledge│         │   │
│  │ │ tools    │ │ tools    │ │ tools    │ │ tools    │         │   │
│  │ └──────────┘ └──────────┘ └──────────┘ └──────────┘         │   │
│  │ ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐         │   │
│  │ │ routing  │ │ report   │ │ config   │ │ trunk    │         │   │
│  │ │ tools    │ │ tools    │ │ tools    │ │ tools    │         │   │
│  │ └──────────┘ └──────────┘ └──────────┘ └──────────┘         │   │
│  └──────────────────────────────────────────────────────────────┘   │
├─────────────────────────────────────────────────────────────────────┤
│                     CLI / SDK Layer                                   │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐               │
│  │ rvoip    │ │ Python   │ │ Node.js  │ │ Rust     │               │
│  │ CLI      │ │ SDK      │ │ SDK      │ │ SDK      │               │
│  └──────────┘ └──────────┘ └──────────┘ └──────────┘               │
├─────────────────────────────────────────────────────────────────────┤
│                  Core Platform (已有)                                 │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐               │
│  │ call     │ │ registrar│ │ session  │ │ web      │               │
│  │ engine   │ │ core     │ │ core     │ │ console  │               │
│  └──────────┘ └──────────┘ └──────────┘ └──────────┘               │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐                             │
│  │ SIP/RTP  │ │ PostgreSQL│ │ WS/HTTP │                             │
│  │ UDP/TCP  │ │ 18       │ │ API     │                             │
│  └──────────┘ └──────────┘ └──────────┘                             │
└─────────────────────────────────────────────────────────────────────┘
```

## 二、MCP Server — 让 AI 直接操作呼叫中心

### 什么是 MCP？

MCP (Model Context Protocol) 是 Anthropic 定义的标准协议，让 AI 模型（Claude、GPT 等）通过 Tool Use 直接调用外部系统。rvoip 实现 MCP Server 后，AI 就能：

- "帮我查看当前有多少活跃通话" → 调用 `get_active_calls` tool
- "把 1001 分机的坐席设为忙碌" → 调用 `update_agent_status` tool
- "生成本周的坐席绩效报表" → 调用 `generate_report` tool
- "创建一个新的 IVR 菜单" → 调用 `create_ivr_menu` tool

### MCP Tools 定义

```
rvoip-mcp-server/
├── tools/
│   ├── calls/
│   │   ├── list_active_calls     — 列出活跃通话
│   │   ├── get_call_detail       — 获取通话详情
│   │   ├── hangup_call           — 挂断通话
│   │   ├── transfer_call         — 转接通话
│   │   ├── get_call_history      — 查询通话历史
│   │   └── get_call_stats        — 获取通话统计
│   │
│   ├── agents/
│   │   ├── list_agents           — 列出坐席
│   │   ├── create_agent          — 创建坐席
│   │   ├── update_agent          — 更新坐席
│   │   ├── set_agent_status      — 设置坐席状态
│   │   ├── get_agent_performance — 获取坐席绩效
│   │   └── assign_agent_skills   — 分配技能
│   │
│   ├── queues/
│   │   ├── list_queues           — 列出队列
│   │   ├── get_queue_status      — 获取队列状态
│   │   ├── create_queue          — 创建队列
│   │   ├── assign_call           — 手动分配通话
│   │   └── get_queue_stats       — 队列统计
│   │
│   ├── routing/
│   │   ├── get_routing_config    — 获取路由配置
│   │   ├── update_routing        — 更新路由策略
│   │   └── test_routing          — 测试路由规则
│   │
│   ├── knowledge/
│   │   ├── search_articles       — 搜索知识库
│   │   ├── get_article           — 获取文章
│   │   ├── get_talk_script       — 获取话术
│   │   └── suggest_response      — 推荐回复话术
│   │
│   ├── system/
│   │   ├── get_system_health     — 系统健康
│   │   ├── get_dashboard         — 仪表盘数据
│   │   ├── get_audit_log         — 审计日志
│   │   └── export_config         — 导出配置
│   │
│   └── ai/
│       ├── transcribe_call       — 通话实时转写
│       ├── analyze_sentiment     — 情绪分析
│       ├── summarize_call        — 通话摘要
│       ├── suggest_action        — 推荐操作
│       └── evaluate_quality      — AI 质检评分
│
├── resources/
│   ├── calls://active            — 活跃通话列表
│   ├── agents://online           — 在线坐席
│   ├── queues://status           — 队列状态
│   ├── knowledge://articles      — 知识库文章
│   └── config://current          — 当前配置
│
└── prompts/
    ├── call_center_assistant     — 呼叫中心管理助手
    ├── quality_reviewer          — 质检评审员
    └── report_analyst            — 报表分析师
```

### MCP Server 实现架构

```rust
// crates/mcp-server/src/lib.rs
// 基于 Claude MCP SDK (Rust)

pub struct RvoipMcpServer {
    engine: Arc<CallCenterEngine>,
    registrar: Arc<RegistrarService>,
    auth_service: Arc<AuthenticationService>,
    knowledge_db: PgPool,
}

impl McpServer for RvoipMcpServer {
    async fn list_tools(&self) -> Vec<Tool> {
        vec![
            Tool::new("list_active_calls", "List all active calls in the call center")
                .with_parameter("status", "Filter by status", false),
            Tool::new("get_agent_performance", "Get performance metrics for an agent")
                .with_parameter("agent_id", "Agent ID", true)
                .with_parameter("period", "Time period (today/week/month)", false),
            // ... 30+ tools
        ]
    }

    async fn call_tool(&self, name: &str, args: Value) -> Result<Value> {
        match name {
            "list_active_calls" => self.handle_list_calls(args).await,
            "get_agent_performance" => self.handle_agent_perf(args).await,
            // ...
        }
    }

    async fn list_resources(&self) -> Vec<Resource> {
        vec![
            Resource::new("calls://active", "Active calls", "application/json"),
            Resource::new("agents://online", "Online agents", "application/json"),
        ]
    }
}
```

## 三、AI Copilot — 坐席实时助手

### 通话中 AI 辅助

```
┌─────────────────────────────────────────────────┐
│ 坐席工作台                                       │
├─────────────────────────────────────────────────┤
│ ┌─────────────────────┐ ┌─────────────────────┐ │
│ │ 通话面板             │ │ AI Copilot          │ │
│ │                     │ │                     │ │
│ │ 客户: 138xxxx001    │ │ 📝 实时转写:        │ │
│ │ 时长: 02:34         │ │ "我的网络连不上..."  │ │
│ │ [保持] [转接] [挂断] │ │                     │ │
│ │                     │ │ 🧠 意图识别:        │ │
│ │ 🟢 通话质量: 良好    │ │ → 网络故障报修      │ │
│ │                     │ │                     │ │
│ │                     │ │ 😊 情绪: 中性→焦虑  │ │
│ │                     │ │                     │ │
│ │                     │ │ 💡 建议回复:        │ │
│ │                     │ │ "我理解您的情况，   │ │
│ │                     │ │  让我帮您检查..."   │ │
│ │                     │ │                     │ │
│ │                     │ │ 📚 相关知识:        │ │
│ │                     │ │ → 网络故障排除指南   │ │
│ │                     │ │ → 路由器重置步骤     │ │
│ │                     │ │                     │ │
│ │                     │ │ ⚠️ 质检提醒:        │ │
│ │                     │ │ "请确认客户身份"    │ │
│ └─────────────────────┘ └─────────────────────┘ │
└─────────────────────────────────────────────────┘
```

### 技术实现

```
音频流 (RTP)
    │
    ▼
ASR (Whisper/Paraformer) — 实时转写
    │
    ├──▶ NLU (Claude Function Calling) — 意图识别
    │
    ├──▶ Sentiment Analysis — 情绪分析
    │
    ├──▶ RAG (知识库检索 + LLM 生成) — 话术建议
    │
    └──▶ Quality Monitor — 实时质检评分
```

## 四、AI-Powered IVR — 对话式语音助手

替换传统按键 IVR，用 AI 语音助手：

```
来电 → ASR 转写 → "我想查一下上个月的账单"
                      │
                      ▼
         LLM 理解意图 → "账单查询"
                      │
                      ▼
         Function Call → query_billing(customer_id, month)
                      │
                      ▼
         LLM 生成回复 → "您上个月的账单金额是 328 元..."
                      │
                      ▼
         TTS 合成语音 → 播放给客户
```

### 与传统 IVR 的对比

| 传统 IVR | AI 语音助手 |
|----------|------------|
| "按1技术支持，按2销售" | "您好，请问有什么可以帮您？" |
| 最多 3-4 层菜单 | 自然对话，无限层级 |
| 只能按键输入 | 语音自然交互 |
| 固定流程 | 动态理解意图 |
| 无法处理复杂问题 | 可以直接回答简单问题 |
| 按键识别率 100% | 语音识别率 95%+ |

## 五、rvoip CLI — 命令行管理工具

```bash
# 安装
cargo install rvoip-cli

# 查看系统状态
$ rvoip status
╔══════════════════════════════════════╗
║ rvoip Call Center Status             ║
╠══════════════════════════════════════╣
║ Active Calls:    12                  ║
║ Online Agents:   8                   ║
║ Queue Depth:     3                   ║
║ SIP Registrations: 24               ║
║ System Health:   ● Healthy           ║
╚══════════════════════════════════════╝

# 坐席管理
$ rvoip agent list
ID        Name      Status     Ext   Department
AGT-001   李伟      Available  1001  技术支持
AGT-002   张明      Busy       1002  销售部
AGT-003   王芳      Offline    1003  客服部

$ rvoip agent create --name "赵六" --department "技术支持"
✅ Created agent AGT-004 (ext: 1004, sip:1004@call-center.local)

$ rvoip agent status AGT-001 --set busy
✅ Agent AGT-001 status changed to Busy

# 通话管理
$ rvoip call list
CALL-ID    From          To           Agent    Status   Duration
call-847   138xxxx001    support      AGT-001  Active   04:32
call-845   139xxxx002    sales        AGT-002  Active   02:15

$ rvoip call hangup call-847
✅ Call call-847 terminated

# 队列管理
$ rvoip queue status
Queue      Waiting  Agents  Avg Wait  SLA
support    2        3       45s       92%
sales      0        2       12s       98%
billing    1        2       30s       95%

# 报表
$ rvoip report daily --date 2026-03-24
📊 Daily Report: 2026-03-24
Total Calls: 156    Answered: 142    Abandoned: 14
Avg Duration: 4:32  Avg Wait: 23s    SLA: 94%

$ rvoip report export --format csv --period week
✅ Exported to rvoip-report-2026-w12.csv

# AI 功能
$ rvoip ai summarize call-847
📝 Call Summary:
Customer reported internet connectivity issues. Agent walked
through router reset procedure. Issue resolved after firmware
update. Customer satisfied.

$ rvoip ai suggest-routing --intent "billing dispute" --language "chinese"
💡 Recommended: Queue 'billing' → Agent with skills [Chinese, Billing, VIP]

# 配置
$ rvoip config show routing
strategy: SkillBased
load_balancing: LeastBusy
geographic_routing: disabled
time_based_routing: enabled

$ rvoip config set routing.strategy RoundRobin
✅ Routing strategy updated to RoundRobin

# MCP 模式 (让 AI 通过 CLI 操作)
$ rvoip mcp serve --port 3001
🤖 MCP Server listening on stdio (or port 3001)
Available tools: 35
Available resources: 5
Available prompts: 3
```

## 六、SDK — 开发者集成

### Python SDK

```python
import rvoip

# 连接
client = rvoip.Client("http://127.0.0.1:3000", api_key="rvoip_xxxxx")

# 查看通话
calls = client.calls.list(status="active")
for call in calls:
    print(f"{call.id}: {call.from_uri} → {call.to_uri} ({call.duration}s)")

# 创建坐席
agent = client.agents.create(
    display_name="新坐席",
    department="技术支持",
    skills=["english", "technical"],
    max_concurrent_calls=3
)
print(f"Created: {agent.id} (ext: {agent.extension})")

# AI 集成
summary = client.ai.summarize_call("call-847")
sentiment = client.ai.analyze_sentiment("call-847")

# 实时事件监听
async for event in client.events.stream():
    if event.type == "call_started":
        print(f"New call: {event.data.from_uri}")
    elif event.type == "agent_status_changed":
        print(f"Agent {event.data.agent_id} → {event.data.new_status}")
```

### Node.js SDK

```javascript
const { RvoipClient } = require('@rvoip/sdk');

const client = new RvoipClient({
  baseUrl: 'http://127.0.0.1:3000',
  apiKey: 'rvoip_xxxxx'
});

// 实时事件
client.on('call:started', (call) => {
  console.log(`New call from ${call.fromUri}`);
});

// AI Copilot WebSocket
const copilot = await client.ai.startCopilot({
  agentId: 'AGT-001',
  features: ['transcription', 'sentiment', 'suggestions']
});

copilot.on('transcription', (text) => {
  console.log(`Customer: ${text}`);
});

copilot.on('suggestion', (suggestion) => {
  console.log(`AI suggests: ${suggestion}`);
});
```

## 七、实施优先级

### Phase A: MCP Server (最高优先级)

让 AI 能操作 rvoip，这是最高杠杆的改动。

```
crates/mcp-server/
├── Cargo.toml       # 依赖 rmcp (Rust MCP SDK)
├── src/
│   ├── lib.rs
│   ├── server.rs    # MCP Server 主循环
│   ├── tools/       # 30+ Tool 实现
│   │   ├── calls.rs
│   │   ├── agents.rs
│   │   ├── queues.rs
│   │   ├── routing.rs
│   │   ├── knowledge.rs
│   │   └── system.rs
│   ├── resources/   # Resource 提供者
│   └── prompts/     # 预置 Prompt
└── examples/
    └── claude_demo.rs  # Claude 集成演示
```

工作量：3-5 天
价值：极高 — 立即让所有 AI 模型能管理呼叫中心

### Phase B: CLI 工具

```
crates/cli/
├── Cargo.toml       # clap + reqwest + tabled
├── src/
│   ├── main.rs
│   ├── commands/    # 子命令
│   │   ├── agent.rs
│   │   ├── call.rs
│   │   ├── queue.rs
│   │   ├── config.rs
│   │   ├── report.rs
│   │   └── ai.rs
│   └── api_client.rs  # HTTP 客户端
└── README.md
```

工作量：2-3 天
价值：高 — 运维必备，脚本化管理

### Phase C: AI Copilot (通话辅助)

依赖：ASR 引擎集成、LLM API 调用
工作量：1-2 周
价值：极高 — 核心竞争力

### Phase D: AI IVR (对话式导航)

依赖：ASR + TTS + LLM
工作量：2-3 周
价值：高 — 替代传统按键 IVR

### Phase E: SDK (Python/Node.js)

工作量：1 周
价值：中 — 开发者生态
