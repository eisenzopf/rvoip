# RVOIP 测试规划

**Document ID**: TEST-006
**Date**: 2026-03-21

---

## 1. 模块分层与测试策略

### 独立模块（可单独测试）
```
sip-core          → 2,002 tests ✅ 充分
codec-core        → 需验证 opus feature
infra-common      → 需补充事件系统测试
```

### 单依赖模块（mock 上层即可测试）
```
sip-transport     → 2 tests ⚠️ 严重不足
rtp-core          → 368 tests ✅ 充分（含适配器）
registrar-core    → 需验证
```

### 核心枢纽（需跨模块集成测试）
```
dialog-core       → 需与 sip-transport 联测
media-core        → 需与 rtp-core 联测
session-core      → 81 test files，但缺 E2E
```

### 应用层（需全链路测试）
```
client-core       → 需 session→dialog→transport 全链路
call-engine       → 2 tests ⚠️ 严重不足
sip-client        → 需端到端验证
```

---

## 2. 六条关键测试路径

### Path A: 出站呼叫 (UAC)
```
client-core → session-core → dialog-core → sip-transport → UDP
                           → media-core → rtp-core → audio-core
```
**测试要求**: 发起 INVITE → 收到 200 OK → 建立 RTP → 发送音频 → BYE 挂断
**现有覆盖**: session-core/tests/ 有基本测试
**缺失**: 完整链路含音频验证

### Path B: 入站呼叫 (UAS)
```
sip-transport → dialog-core → session-core → CallHandler
                            → media-core → rtp-core → audio-core
```
**测试要求**: 接收 INVITE → 180 Ringing → 200 OK → 接收 RTP → 播放音频
**现有覆盖**: 有基本的 UAS 测试
**缺失**: 完整链路含音频验证

### Path C: 加密媒体 (DTLS-SRTP)
```
SDP offer(fingerprint) → DTLS handshake → SRTP key → RTP encrypt/decrypt
```
**测试要求**: SDP 协商 → DTLS 握手 → 密钥提取 → 加密发送 → 解密接收 → 验证音频完整
**现有覆盖**: srtp adapter 有 roundtrip 测试
**缺失**: session-core srtp_bridge → 实际 DTLS 握手 → 加密 RTP 端到端

### Path D: NAT 穿越 (ICE)
```
ICE gather(STUN+TURN) → SDP offer(candidates) → connectivity checks → selected pair → media
```
**测试要求**: 收集候选 → SDP 交换 → 连通性检查 → 选定候选对 → 媒体流通
**现有覆盖**: ICE adapter 15 tests
**缺失**: 真实 STUN 服务器联测、ICE+媒体端到端

### Path E: 注册 (REGISTER + Auth)
```
client-core → session-core → dialog-core → sip-transport
                           → digest auth (401 challenge-response)
```
**测试要求**: REGISTER → 401 → compute digest → REGISTER w/ Authorization → 200 OK → 刷新
**现有覆盖**: 基本注册测试
**缺失**: 实际 SIP 服务器（FreeSWITCH/Asterisk）互操作

### Path F: 呼叫中心 (B2BUA)
```
call-engine → session-core → dialog-core (inbound leg)
           → session-core → dialog-core (outbound leg)
           → media-core (bridge/mix)
```
**测试要求**: 客户呼入 → 排队 → 分配坐席 → 桥接双腿 → 音频混合 → 挂断清理
**现有覆盖**: 2 个测试 ⚠️
**缺失**: 完整 B2BUA 桥接含音频

---

## 3. 测试层级

### Level 1: 单元测试（每个模块独立）
```
cargo test -p rvoip-sip-core --lib
cargo test -p rvoip-rtp-core --lib
cargo test -p rvoip-codec-core --lib
cargo test -p rvoip-media-core --lib
cargo test -p rvoip-dialog-core --lib
cargo test -p rvoip-session-core --lib
cargo test -p rvoip-client-core --lib
cargo test -p rvoip-call-engine --lib
cargo test -p rvoip-infra-common --lib
```

### Level 2: 适配器 roundtrip 测试
```
# 每个迁移适配器的 roundtrip 验证
cargo test -p rvoip-rtp-core --lib -- adapter
cargo test -p rvoip-rtp-core --lib -- ice::adapter
cargo test -p rvoip-rtp-core --lib -- sctp::adapter
cargo test -p rvoip-rtp-core --lib -- srtp::adapter
cargo test -p rvoip-rtp-core --lib -- dtls::adapter
cargo test -p rvoip-rtp-core --lib -- stun::adapter
cargo test -p rvoip-rtp-core --lib -- packet::adapter
```

### Level 3: 跨模块集成测试
```
# dialog-core + sip-transport
# session-core + dialog-core + media-core
# DTLS adapter → SRTP adapter (密钥提取→加密)
# ICE adapter + STUN adapter (候选收集)
```

### Level 4: 端到端路径测试
```
# Path A: 完整出站呼叫
# Path B: 完整入站呼叫
# Path C: 加密媒体全链路
# Path D: NAT 穿越全链路
# Path E: 注册+认证全链路
# Path F: B2BUA 桥接全链路
```

### Level 5: 外部互操作测试
```
# rvoip ↔ FreeSWITCH
# rvoip ↔ Asterisk
# rvoip ↔ WebRTC Browser (Chrome/Firefox)
# rvoip ↔ SIP softphone (Ooh, Ooh Ooh)
```

---

## 4. 优先级修复清单

### P0: 严重不足（< 5 tests 的关键模块）
| 模块 | 当前 | 需新增 | 类型 |
|------|------|--------|------|
| sip-transport | 2 | 10+ | UDP/TCP/TLS/WS 联通测试 |
| call-engine | 2 | 15+ | B2BUA 桥接、排队、路由 |

### P1: 缺失的跨模块集成
| 测试路径 | 需新增 | 说明 |
|----------|--------|------|
| DTLS→SRTP 端到端 | 3+ | 握手→密钥→加密→解密 roundtrip |
| ICE+媒体 | 3+ | 候选收集→SDP→连通→媒体 |
| 完整呼叫+音频 | 2+ | UAC↔UAS 含 G.711 音频验证 |
| B2BUA 双腿桥接 | 3+ | 入站→桥接→出站含音频 |

### P2: 适配器覆盖补全
| 适配器 | 缺失 | 需新增 |
|--------|------|--------|
| RTCP 转换 roundtrip | 是 | 2+ |
| DTLS 握手集成 | 是 | 1+ |
| ICE 候选映射 roundtrip | 是 | 2+ |
| Audio APM 处理验证 | 是 | 2+ (需 webrtc-apm feature) |

---

*End of Test Plan*
