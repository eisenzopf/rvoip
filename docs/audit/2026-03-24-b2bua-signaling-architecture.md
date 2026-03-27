# B2BUA 信令桥接架构审计与修复方案

> **日期**: 2026-03-24
> **审计人**: Claude (待 Codex 交叉审计)
> **状态**: 待审计
> **严重度**: P0 — 核心呼叫功能不可用

---

## 1. 问题概述

rvoip 服务器作为 B2BUA (Back-to-Back User Agent) 处理浏览器间 SIP 呼叫：

```
SIP.js(1001)  ←→  rvoip B2BUA  ←→  SIP.js(1002)
   A-leg              桥接              B-leg
  (UAS端)                             (UAC端)
```

### 当前状态

| 功能 | 状态 | 说明 |
|------|------|------|
| REGISTER | ✅ | 两端注册成功，AOR路由学习正确 |
| INVITE 转发 | ✅ | A-leg收到INVITE → Forward决策 → B-leg发出INVITE |
| 被叫响铃 | ✅ | 1002的SIP.js收到INVITE并显示来电 |
| 被叫接听 | ❌ | B-leg 200 OK收到，但A-leg没收到200 OK |
| 主叫状态更新 | ❌ | 1001一直显示"呼叫中"，不会变成"通话中" |
| 挂断转发 | ❌ | 任一方BYE不传递到另一方 |
| 不存在号码拒绝 | ✅ | 发送480 Temporarily Unavailable |

### 三个根本缺陷

1. **无桥接状态管理** — 没有数据结构追踪 A-leg ↔ B-leg 对应关系
2. **B-leg事件不转发** — B-leg的200 OK/180 Ringing/BYE不传递给A-leg
3. **事件系统双轨运行** — GlobalEventCoordinator + legacy mpsc channel 并行，事件丢失

---

## 2. 当前事件流分析

### 2.1 事件发布路径（dialog-core → session-core）

```
dialog-core SessionCoordinationEvent
        │
        ↓ try_emit_session_coordination_event()
        │
        ├─[1] event_hub.publish_session_coordination_event()
        │       │
        │       ├─ convert_coordination_to_cross_crate()
        │       │   ├─ IncomingCall    → DialogToSession::IncomingCall  ✅ 有映射
        │       │   ├─ CallAnswered    → None                          ❌ 无映射
        │       │   ├─ CallRinging     → None                          ❌ 无映射
        │       │   ├─ CallTerminating → None                          ❌ 无映射
        │       │   ├─ CallTerminated  → None                          ❌ 无映射
        │       │   ├─ ResponseReceived→ None                          ❌ 无映射
        │       │   ├─ AckSent         → None                          ❌ 无映射
        │       │   └─ AckReceived     → None                          ❌ 无映射
        │       │
        │       └─ GlobalEventCoordinator.publish()
        │           └─ broadcast["dialog_to_session"] → session-core订阅 ✅
        │
        └─[2] legacy mpsc channel → dialog_coordinator.start_event_loop()
                │
                ├─ handle_incoming_call()      ✅ 但和[1]重复
                ├─ handle_call_answered()       ✅ 能收到
                ├─ handle_call_terminating()    ✅ 能收到
                ├─ handle_response_received()   ✅ 能收到
                └─ handle_ack_sent()            ✅ 能收到
```

**问题**：两条路径并行。IncomingCall走[1]到cross-crate bus，其余走[2]到legacy channel。
但[2]的handler不知道B2BUA桥接关系，只做单session处理。

### 2.2 B-leg 200 OK 事件路径（当前·失败的）

```
SIP.js(1002) 发送 200 OK
    │
    ↓ dialog-core 收到 200 OK response
    │
    ├─ Dialog Early → Confirmed
    ├─ 自动发送 ACK
    │
    ├─ emit CallAnswered { dialog_id: B-leg-dialog, sdp_answer }
    │   │
    │   ├─ event_hub: convert → None (无跨crate映射)
    │   ├─ event_hub: 返回 Ok(()) (非REGISTER/OPTIONS，不返回Err)
    │   │
    │   ├─ try_emit: event_hub成功 → return Ok ← 问题! 不走legacy了
    │   │                                         (已修复为双发)
    │   │
    │   └─ legacy channel → dialog_coordinator.handle_call_answered()
    │       │
    │       ├─ 找到 B-leg session ✅
    │       ├─ 存储 SDP ✅
    │       └─ 尝试B2BUA转发... ← 但不知道A-leg是谁! ❌
    │
    └─ A-leg 的 INVITE server transaction 一直等待 → 超时
```

### 2.3 BYE 事件路径（当前·失败的）

```
SIP.js(1001) 发送 BYE    或    SIP.js(1002) 发送 BYE
    │                               │
    ↓                               ↓
dialog-core 处理BYE             dialog-core 处理BYE
    │                               │
    ├─ 200 OK 回复                  ├─ 200 OK 回复
    ├─ Dialog → Terminated          ├─ Dialog → Terminated
    │                               │
    ├─ emit CallTerminating         ├─ emit CallTerminating
    │   └─ legacy channel →         │   └─ legacy channel →
    │       handle_call_terminating │       handle_call_terminating
    │       │                       │       │
    │       ├─ 更新session状态 ✅   │       ├─ 更新session状态 ✅
    │       └─ 不知道对方! ❌       │       └─ 不知道对方! ❌
    │                               │
    └─ 对方SIP.js不知道 ❌          └─ 对方SIP.js不知道 ❌
```

---

## 3. 架构修复方案

### 3.1 核心设计：BridgeManager

引入 `BridgeManager` 作为B2BUA桥接的单一数据源，追踪每对A/B-leg的完整生命周期。

```
                    ┌──────────────────────┐
                    │    BridgeManager     │
                    │                      │
                    │  bridges: DashMap    │
                    │    bridge_id → {     │
                    │      a_leg_session   │
                    │      b_leg_session   │
                    │      a_leg_dialog    │
                    │      b_leg_dialog    │
                    │      state           │
                    │    }                 │
                    │                      │
                    │  session_to_bridge:  │
                    │    DashMap           │
                    │    session → bridge  │
                    └──────────────────────┘
                              │
                 ┌────────────┼────────────┐
                 │            │            │
          ┌──────────┐ ┌──────────┐ ┌──────────┐
          │ Forward  │ │ Answer   │ │  BYE     │
          │ handler  │ │ handler  │ │ handler  │
          └──────────┘ └──────────┘ └──────────┘
```

#### 数据结构

```rust
/// crates/session-core/src/b2bua/bridge_manager.rs

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeState {
    /// B-leg INVITE已发出，等待响应
    AwaitingResponse,
    /// B-leg 180 Ringing，已转发给A-leg
    Ringing,
    /// B-leg 200 OK，A-leg已接受
    Established,
    /// 一方发送BYE，正在终止另一方
    Terminating { initiator: SessionId },
    /// 双方都已终止
    Terminated,
}

#[derive(Debug, Clone)]
pub struct Bridge {
    pub id: String,
    pub a_leg: LegInfo,
    pub b_leg: LegInfo,
    pub state: BridgeState,
    pub created_at: Instant,
}

#[derive(Debug, Clone)]
pub struct LegInfo {
    pub session_id: SessionId,
    pub dialog_id: DialogId,
}
```

#### BridgeManager API

```rust
pub struct BridgeManager {
    bridges: DashMap<String, Bridge>,
    session_to_bridge: DashMap<SessionId, String>,
}

impl BridgeManager {
    /// 创建桥接（Forward决策后调用）
    pub fn create_bridge(&self, a_leg: LegInfo, b_leg: LegInfo) -> String;

    /// 查找session所在的桥接
    pub fn get_bridge_for_session(&self, session_id: &SessionId) -> Option<Bridge>;

    /// 获取桥接伙伴的信息
    pub fn get_partner(&self, session_id: &SessionId) -> Option<LegInfo>;

    /// 更新桥接状态
    pub fn set_state(&self, bridge_id: &str, state: BridgeState);

    /// 销毁桥接
    pub fn destroy(&self, bridge_id: &str);
}
```

### 3.2 事件处理重构

#### 原则：所有B2BUA信令逻辑集中在一个模块

```
crates/session-core/src/
├── b2bua/                         ← 新增模块
│   ├── mod.rs
│   ├── bridge_manager.rs          ← BridgeManager 数据结构
│   └── signaling.rs               ← B2BUA 信令桥接逻辑
├── coordinator/
│   └── event_handler.rs           ← 调用 b2bua 模块
└── dialog/
    └── coordinator.rs             ← 不再处理B2BUA逻辑
```

#### signaling.rs 核心逻辑

```rust
/// crates/session-core/src/b2bua/signaling.rs
///
/// B2BUA信令桥接：所有跨leg信令转发在此处理

pub struct B2BuaSignaling {
    bridge_manager: Arc<BridgeManager>,
    dialog_manager: Arc<DialogManager>,  // session-core的
}

impl B2BuaSignaling {
    /// 处理Forward决策：创建B-leg并建立桥接
    ///
    /// 调用时机：CallDecision::Forward 返回后
    /// 输入：A-leg session/dialog, target URI
    /// 输出：Bridge创建成功/失败
    pub async fn handle_forward(
        &self,
        a_leg_session_id: &SessionId,
        a_leg_dialog_id: &DialogId,
        target_uri: &str,
        from_uri: &str,
        coordinator: &SessionCoordinator,
    ) -> Result<String> {
        // 1. 创建B-leg outgoing call
        let b_leg = coordinator.create_outgoing_call(from_uri, target_uri, None, None).await?;
        let b_leg_session_id = b_leg.id().clone();
        let b_leg_dialog_id = self.dialog_manager.get_dialog_id_for_session(&b_leg_session_id)?;

        // 2. 创建桥接
        let bridge_id = self.bridge_manager.create_bridge(
            LegInfo { session_id: a_leg_session_id.clone(), dialog_id: a_leg_dialog_id.clone() },
            LegInfo { session_id: b_leg_session_id, dialog_id: b_leg_dialog_id },
        );

        tracing::info!("B2BUA: Bridge {} created: A={} ↔ B={}", bridge_id,
            a_leg_session_id, b_leg.id());

        Ok(bridge_id)
    }

    /// B-leg 收到 180 Ringing → 给 A-leg 发 180
    ///
    /// 调用时机：dialog_coordinator 收到 CallRinging 事件
    pub async fn handle_b_leg_ringing(&self, b_leg_session_id: &SessionId) -> Result<()> {
        let bridge = self.bridge_manager.get_bridge_for_session(b_leg_session_id)
            .ok_or_else(|| SessionError::internal("Session not in bridge"))?;

        // 给A-leg的INVITE server transaction发180 Ringing
        let a_dialog_id = &bridge.a_leg.dialog_id;
        self.dialog_manager.send_provisional_response(a_dialog_id, 180, "Ringing").await?;

        self.bridge_manager.set_state(&bridge.id, BridgeState::Ringing);
        tracing::info!("B2BUA: Forwarded 180 Ringing to A-leg {}", bridge.a_leg.session_id);
        Ok(())
    }

    /// B-leg 收到 200 OK → 给 A-leg 发 200 OK
    ///
    /// 调用时机：dialog_coordinator 收到 CallAnswered 事件
    /// 这是B2BUA最关键的信令桥接点
    pub async fn handle_b_leg_answered(
        &self,
        b_leg_session_id: &SessionId,
        sdp_answer: &str,
    ) -> Result<()> {
        let bridge = self.bridge_manager.get_bridge_for_session(b_leg_session_id)
            .ok_or_else(|| SessionError::internal("Session not in bridge"))?;

        // 给A-leg的INVITE发200 OK（携带B-leg的SDP answer）
        let a_dialog_id = &bridge.a_leg.dialog_id;
        self.dialog_manager.accept_incoming_call(
            &bridge.a_leg.session_id,
            Some(sdp_answer.to_string()),
        ).await?;

        self.bridge_manager.set_state(&bridge.id, BridgeState::Established);
        tracing::info!("B2BUA: Bridge {} established — A-leg {} ↔ B-leg {}",
            bridge.id, bridge.a_leg.session_id, bridge.b_leg.session_id);
        Ok(())
    }

    /// 任一方发送 BYE → 转发给另一方
    ///
    /// 调用时机：dialog_coordinator 收到 CallTerminating 事件
    pub async fn handle_leg_terminating(
        &self,
        terminating_session_id: &SessionId,
        reason: &str,
    ) -> Result<()> {
        let bridge = match self.bridge_manager.get_bridge_for_session(terminating_session_id) {
            Some(b) => b,
            None => return Ok(()), // 不在桥接中，正常单session终止
        };

        // 防止循环终止
        if matches!(bridge.state, BridgeState::Terminating { .. } | BridgeState::Terminated) {
            tracing::debug!("B2BUA: Bridge {} already terminating, skip", bridge.id);
            return Ok(());
        }

        // 标记桥接为终止中
        self.bridge_manager.set_state(&bridge.id, BridgeState::Terminating {
            initiator: terminating_session_id.clone(),
        });

        // 找到对方
        let partner = self.bridge_manager.get_partner(terminating_session_id)
            .ok_or_else(|| SessionError::internal("Bridge partner not found"))?;

        tracing::info!("B2BUA: {} terminated, sending BYE to partner {}",
            terminating_session_id, partner.session_id);

        // 给对方发BYE
        if let Err(e) = self.dialog_manager.terminate_session(&partner.session_id).await {
            tracing::error!("B2BUA: Failed to terminate partner {}: {}", partner.session_id, e);
        }

        // 清理桥接
        self.bridge_manager.set_state(&bridge.id, BridgeState::Terminated);
        self.bridge_manager.destroy(&bridge.id);

        Ok(())
    }

    /// Forward失败 → 给A-leg发SIP错误响应
    pub async fn handle_forward_failed(
        &self,
        a_leg_session_id: &SessionId,
        error: &str,
    ) -> Result<()> {
        // 选择合适的SIP状态码
        let status = if error.contains("routing") || error.contains("resolve") {
            rvoip_sip_core::StatusCode::TemporarilyUnavailable // 480
        } else {
            rvoip_sip_core::StatusCode::ServerInternalError     // 500
        };

        self.dialog_manager.reject_incoming_session(
            a_leg_session_id, status, Some(error.to_string()),
        ).await?;

        Ok(())
    }
}
```

### 3.3 事件处理器集成

```rust
/// crates/session-core/src/coordinator/event_handler.rs
///
/// 修改 CallDecision::Forward 处理

CallDecision::Forward(target) => {
    let clean_from = clean_sip_uri(&from);
    let clean_target = clean_sip_uri(&target);

    match self.b2bua_signaling.handle_forward(
        &session_id,
        &dialog_id,
        &clean_target,
        &clean_from,
        &self,
    ).await {
        Ok(bridge_id) => {
            tracing::info!("B2BUA: Bridge {} created for call {}", bridge_id, session_id);
        }
        Err(e) => {
            tracing::error!("B2BUA: Forward failed: {}", e);
            self.b2bua_signaling.handle_forward_failed(&session_id, &e.to_string()).await.ok();
        }
    }
}
```

### 3.4 dialog_coordinator 集成

```rust
/// crates/session-core/src/dialog/coordinator.rs
///
/// 修改已有的 handle_* 方法，添加B2BUA钩子

async fn handle_call_answered(&self, dialog_id: DialogId, sdp: String) -> DialogResult<()> {
    if let Some(session_id) = self.dialog_to_session.get(&dialog_id).map(|r| r.clone()) {
        // 存储SDP
        // ...

        // B2BUA钩子：如果此session在桥接中，转发200 OK给A-leg
        if let Err(e) = self.b2bua_signaling.handle_b_leg_answered(&session_id, &sdp).await {
            tracing::warn!("B2BUA answer forwarding failed (may not be in bridge): {}", e);
        }
    }
    Ok(())
}

async fn handle_call_terminating(&self, dialog_id: DialogId, reason: String) -> DialogResult<()> {
    if let Some(session_id) = self.dialog_to_session.get(&dialog_id).map(|r| r.clone()) {
        // 更新session状态
        // ...

        // B2BUA钩子：如果此session在桥接中，转发BYE给对方
        if let Err(e) = self.b2bua_signaling.handle_leg_terminating(&session_id, &reason).await {
            tracing::warn!("B2BUA termination forwarding failed (may not be in bridge): {}", e);
        }
    }
    Ok(())
}
```

---

## 4. 完整呼叫流程（修复后）

### 4.1 正常呼叫流

```
1001(SIP.js)          rvoip B2BUA              1002(SIP.js)
    │                     │                        │
    │── INVITE ──────────>│                        │
    │                     │ [IncomingCall event]    │
    │                     │ [CallDecision::Forward] │
    │                     │                        │
    │                     │── INVITE ──────────────>│
    │                     │                        │
    │                     │<───────── 180 Ringing ──│
    │<── 180 Ringing ─────│ [B2BUA: forward 180]   │
    │                     │                        │
    │                     │<───────── 200 OK ───────│
    │                     │ [B2BUA: handle_b_leg_answered]
    │<── 200 OK ──────────│ [accept A-leg with B-leg SDP]
    │                     │                        │
    │── ACK ─────────────>│                        │
    │                     │                        │
    │  ════════════ 通话中 ════════════            │
    │                     │                        │
    │── BYE ─────────────>│                        │
    │<── 200 OK ──────────│                        │
    │                     │── BYE ────────────────>│
    │                     │<───────── 200 OK ──────│
    │                     │                        │
```

### 4.2 不存在号码

```
1001(SIP.js)          rvoip B2BUA
    │                     │
    │── INVITE ──────────>│
    │                     │ [Forward → resolve fails]
    │<── 480 ─────────────│ [reject A-leg with 480]
    │                     │
```

### 4.3 被叫拒接

```
1001(SIP.js)          rvoip B2BUA              1002(SIP.js)
    │                     │                        │
    │── INVITE ──────────>│── INVITE ─────────────>│
    │                     │<───────── 486 Busy ────│
    │<── 486 Busy ────────│ [B2BUA: forward reject]│
    │                     │                        │
```

---

## 5. 文件修改清单

| 文件 | 操作 | 说明 |
|------|------|------|
| `session-core/src/b2bua/mod.rs` | 新建 | 模块声明 |
| `session-core/src/b2bua/bridge_manager.rs` | 新建 | BridgeManager数据结构 |
| `session-core/src/b2bua/signaling.rs` | 新建 | B2BUA信令桥接逻辑 |
| `session-core/src/coordinator/coordinator.rs` | 修改 | 添加bridge_manager和b2bua_signaling字段 |
| `session-core/src/coordinator/event_handler.rs` | 修改 | Forward路径使用b2bua模块 |
| `session-core/src/dialog/coordinator.rs` | 修改 | handle_call_answered/terminating添加B2BUA钩子 |
| `session-core/src/dialog/manager.rs` | 修改 | 添加send_provisional_response方法 |
| `dialog-core/src/api/unified.rs` | 修改 | 暴露send_provisional_response |
| `dialog-core/src/manager/core.rs` | 已修改 | 双发event_hub+legacy channel |

---

## 6. 待Codex审计的问题

1. **BridgeManager应该放在哪层？** — session-core还是单独crate？
2. **事件系统统一时机** — 是否应该在本次修复中移除legacy channel？
3. **媒体桥接** — 当前只处理信令，媒体层(RTP)的桥接方案是什么？B2BUA需要RTP relay还是direct media？
4. **SRTP/DTLS** — SIP.js用WebRTC(DTLS-SRTP)，rvoip的RTP层能否与之互通？
5. **竞态条件** — handle_b_leg_answered和handle_leg_terminating的并发安全性
6. **错误恢复** — B-leg超时/5xx时如何通知A-leg？
7. **dialog_coordinator vs event_handler** — 两个事件处理入口的职责边界是否清晰？
