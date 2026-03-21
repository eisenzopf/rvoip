# Codex Audit of Implementation Plan PLAN-004

**Document ID**: AUDIT-005
**Date**: 2026-03-21
**Auditor**: OpenAI Codex (gpt-5.3-codex)
**Subject**: PLAN-004 (SCTP, TURN→ICE, ICE Improvements, Dialog Forking)

---

## Summary Verdict

| Phase | Feature | Verdict | Key Issue |
|-------|---------|---------|-----------|
| 1 | TURN → ICE | **REVISE** | TurnServerConfig 应在 rtp-core 定义；media relay 路径未充分设计 |
| 2 | ICE Keepalive + Trickle | **REVISE** | Consent 应用 Binding Request 非 Indication；INFO 包协商缺失 |
| 3 | Dialog Forking | **REVISE** | Transaction→dialog 查找需先修复；CANCEL 时序不准确 |
| 4 | DTLS-SCTP | **REJECT** | DTLS app-data API 不存在（dtls/connection.rs:842 未实现） |

---

## Phase 1: TURN → ICE (REVISE)

### 确认可行
- `TurnClient::with_socket()` 存在 ✅ (turn/client.rs:119)
- `TurnAllocation` 字段匹配 ✅ (turn/client.rs:52)
- gather.rs 可扩展 ✅

### 需修改
1. **`TurnServerConfig` 应定义在 rtp-core**（非 session-core），避免反向依赖
2. **仅实现 UDP TURN**，TURN TCP/TLS 延后（当前实现仅 UDP: turn/client.rs:167 `TRANSPORT_UDP`）
3. **media relay 路径**: media-core 的 `send_packet` 直接发 UDP，不经过 TURN。需添加 TURN relay 抽象层
4. **共享 socket**: ICE gather 绑定新 socket (agent.rs:164)，应复用 RTP session socket (manager.rs:1882 `get_rtp_socket()`)
5. **`related_address`**: relay candidate 的 related address 应为 server-reflexive 地址，不是 mapped_address（语义不同）

### 修订后分两步
- **Step 1**: 添加 relay candidate 收集（不含 media relay）
- **Step 2**: 添加 TURN media relay send/recv 路径

---

## Phase 2: ICE Keepalive + Trickle (REVISE)

### Consent Freshness (2a) 修正
- **原方案错误**: 计划用 STUN Binding Indication（无响应，类型 0x0011）
- **正确做法**: RFC 7675 要求用 **authenticated Binding Request**（有响应），复用 checklist.rs:116 的 STUN builder
- 无响应 30s → 标记 Disconnected ✅ 这部分正确

### Trickle ICE (2b) 修正
- **原方案不足**: SIP INFO 需要 INFO-package 协商（RFC 6086），不能直接发送
- **修订建议**: 先实现 consent freshness，trickle ICE 延后到 INFO-package 支持就绪
- `a=ice-options:trickle` 和 `a=end-of-candidates` 解析已存在 ✅ (sdp/attributes/ice.rs)
- `add_remote_candidate()` 已存在 ✅ (agent.rs:194)

### 修订后优先级
1. Consent freshness（独立，无协议依赖）
2. Trickle ICE（需 INFO-package 前置）

---

## Phase 3: Dialog Forking (REVISE)

### 确认可行
- DashMap 支持并发访问 ✅
- Early dialog 概念存在 (dialog_operations.rs)

### 需修改
1. **先修复 transaction→dialog 查找**: core.rs:815 和 message_routing.rs:48 存在递归关联问题
2. **CANCEL 时序**: 计划中 "BYE or CANCEL for provisional" 不准确。收到 2xx 后 CANCEL 无效（INVITE transaction 已完成），应只发 BYE
3. **多 2xx 处理**: UAC 必须 ACK 每个 2xx，然后 BYE 不想要的分支
4. **DashMap<String, Vec<DialogId>>** 并发修改弱 → 建议用 guarded branch object 或 DashMap<String, DashSet<DialogId>>
5. 利用已有 `routing/response_router.rs` 的 forking 意图（目前是 TODO stub）

### 修订后分三步
1. 修复 transaction→dialog lookup
2. 实现多 2xx 处理（ACK all + BYE extras）
3. 添加 early-media multi-branch 编排

---

## Phase 4: DTLS-SCTP (REJECT)

### 阻断原因
- **DTLS app-data API 未实现**: dtls/connection.rs:842 显式标记未实现
- **DTLS 构造器未完成**: dtls/mod.rs:84 顶层构造器未实现
- **webrtc-sctp 兼容性未验证**

### 前置条件
1. 实现 DTLS application-data send/receive API
2. 稳定 DTLS transport 生命周期（启动/读取循环）
3. 然后才能评估 webrtc-sctp 集成

### 建议
**延后至 DTLS 基础设施完善后单独立项**

---

## 修订后执行顺序

| 序号 | 功能 | 优先级 | 前置 | 代理数 |
|------|------|--------|------|--------|
| 1 | TURN → ICE relay candidates (UDP only) | HIGH | 无 | 1 |
| 2 | ICE Consent Freshness (RFC 7675) | HIGH | 无 | 1 |
| 3 | Dialog Forking (多 2xx 处理) | MEDIUM | 修复 tx→dialog lookup | 1 |
| 4 | Trickle ICE (RFC 8838) | LOW | INFO-package 支持 | 1 |
| 5 | DTLS-SCTP Data Channels | DEFERRED | DTLS app-data API | — |

---

## 已有可复用代码

| 代码 | 位置 | 用途 |
|------|------|------|
| IceConfig.turn_servers | media/types.rs:107 | 已有字段，未使用 |
| drain_pending() | turn/client.rs:456 | 共享 socket 缓冲 |
| SDP trickle 属性 | sdp/builder.rs:587 | ice-options 生成 |
| SDP end-of-candidates | sdp/attributes/ice.rs:101 | 解析已实现 |
| response_router.rs | dialog-core/routing/ | Forking 意图 stub |
| checklist STUN builder | ice/checklist.rs:116 | 可复用于 consent checks |

---

*End of Codex Plan Audit*
