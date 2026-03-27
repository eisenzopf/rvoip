# Transport Routing 审计报告

> **日期**: 2026-03-24
> **问题**: 200 OK / ACK 通过 UDP 而非 WebSocket 发送

## 现象

| SIP消息 | 预期Transport | 实际Transport | 状态 |
|---------|-------------|-------------|------|
| 100 Trying | WebSocket | WebSocket | ✅ |
| REGISTER 200 OK | WebSocket | WebSocket | ✅ |
| INVITE 200 OK | WebSocket | UDP? | ❌ 待确认 |
| ACK (B-leg) | WebSocket | UDP | ❌ 确认 |

## 根因分析

### ACK 发送（已确认错误）

`crates/dialog-core/src/transaction/manager/creation.rs:237-241`:

```rust
// 直接使用 self.transport (默认 UDP)，绕过 transport_manager
self.transport.send_message(Message::Request(ack_request), destination).await
```

### 200 OK 发送路径

Server Transaction 创建时通过 transport_manager 获取正确的 transport：

```rust
// creation.rs:375-380
let tx_transport = if let Some(ref tm) = self.transport_manager {
    tm.get_transport_for_destination(remote_addr).await
        .unwrap_or_else(|| self.transport.clone())  // ← fallback 到 UDP
} else {
    self.transport.clone()  // ← 如果 transport_manager 为 None
};
```

200 OK 通过 `ServerTransactionData.transport` 发送（和 100 Trying 相同）。
如果 transport_manager 正确初始化且 peer_transport_map 有映射，应该正确。

**需要验证**：transport_manager 在运行时是否为 None。

## 修复方案

### 1. ACK 发送使用 transport_manager
### 2. 添加 transport 选择日志确认 200 OK 路径
### 3. 确保所有 `self.transport` 直接使用的地方都经过 transport_manager
