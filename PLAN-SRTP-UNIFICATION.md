# PLAN: SRTP 架构统一 — SecurityRtpTransport 作为唯一加密层

## 核心思路

```
当前：两套 SRTP 实现，互不相通，实际明文传输
目标：SecurityRtpTransport 作为唯一 SRTP 层，透明加解密

关键洞察：
- RtpSession 已经接受 Arc<dyn RtpTransport> — 不关心具体类型
- SecurityRtpTransport 已经实现完整的 RtpTransport trait + SRTP 加解密
- RtpSession::new() 内部创建 UdpRtpTransport — 需要新构造函数接受外部 transport
- DTLS 握手后的密钥需要传递给 SecurityRtpTransport::set_srtp_context()
```

## 数据流变化

```
修复前 (明文):
  AudioTransmitter → RtpSession → UdpRtpTransport → socket.send_to(明文)
  socket.recv_from(明文) → UdpRtpTransport → RtpSession → 解码

修复后 (加密):
  AudioTransmitter → RtpSession → SecurityRtpTransport → protect_rtp() → socket.send_to(密文)
  socket.recv_from(密文) → SecurityRtpTransport → unprotect_rtp() → RtpSession → 解码
```

## 实施阶段

### Phase 1: rtp-core — 添加外部 transport 注入能力

**文件**: `crates/rtp-core/src/session/mod.rs`

**变更**: 添加 `RtpSession::with_transport()` 构造函数

```rust
/// Create an RtpSession with an externally provided transport.
/// Used when SRTP is needed — caller wraps UdpRtpTransport in SecurityRtpTransport.
pub async fn with_transport(
    config: RtpSessionConfig,
    transport: Arc<dyn RtpTransport>,
) -> Result<Self>
```

- 复用现有 `new()` 的逻辑，但跳过内部 `UdpRtpTransport::new()`
- 使用传入的 `transport` 参数
- 调用 `start()` 启动 send/receive 任务

**文件**: `crates/rtp-core/src/srtp/mod.rs` 或 `context.rs`

**变更**: 确保 `SrtpContext` 可以从原始密钥材料创建（供 session-core 调用）

```rust
pub fn from_key_material(
    master_key: &[u8],
    master_salt: &[u8],
    profile: SrtpProfile,
    is_sender: bool,
) -> Result<Self>
```

### Phase 2: media-core — 支持外部 transport 创建 RTP 会话

**文件**: `crates/media-core/src/relay/controller/mod.rs`

**变更**: `MediaSessionController` 添加带 transport 参数的方法

```rust
/// Start media with an externally provided RTP transport.
/// Used by session-core when SecurityRtpTransport is needed for SRTP.
pub async fn start_media_with_transport(
    &self,
    dialog_id: DialogId,
    config: MediaConfig,
    transport: Arc<dyn RtpTransport>,
) -> Result<()>
```

- 复用 `start_media()` 逻辑但使用 `RtpSession::with_transport()` 而非 `RtpSession::new()`
- 原有 `start_media()` 保持不变（向后兼容非 SRTP 场景）

### Phase 3: session-core — 统一 SRTP 流程

**文件**: `crates/session-core/src/media/manager.rs`

**变更**: 重写 SRTP 会话创建流程

```
新流程:
1. setup_srtp_from_sdp() 检测 SDP 是否需要 SRTP → 存储参数
2. 创建 UdpRtpTransport (用于底层 UDP)
3. 如果需要 SRTP: 用 SecurityRtpTransport 包裹 UdpRtpTransport
4. 通过 MediaSessionController::start_media_with_transport() 创建 RTP 会话
5. perform_dtls_handshake() 使用 UDP socket 完成 DTLS
6. 从 DTLS 提取密钥 → 转换为 SrtpContext
7. 调用 SecurityRtpTransport::set_srtp_context() 安装密钥
8. 此后所有 RTP 收发自动加解密（透明）
```

**需要保留**:
- `SrtpMediaBridge` 的 DTLS 握手逻辑（提取密钥）
- `srtp_required_sessions` 追踪（用于 coordinator 层安全检查）

**可以移除**:
- `send_rtp_with_srtp()` — dormant，不再需要
- `receive_rtp_with_srtp()` — dormant，不再需要
- `protect_rtp()` / `unprotect_rtp()` — SecurityRtpTransport 已处理
- SrtpMediaBridge 中的 `protect_rtp()` / `unprotect_rtp()` — 不再需要

**需要新增**:
- `security_transports: HashMap<SessionId, Arc<SecurityRtpTransport>>` — 追踪每个会话的 SecurityRtpTransport 引用
- 密钥提取辅助函数：从 SrtpMediaBridge 的 DTLS 结果 → SrtpContext

### Phase 4: 清理 TODO 标记和验证

- 移除 `audio_generation.rs` 中的 TODO 标记
- 移除 `udp.rs` 中的 TODO 标记
- 移除 `rtp_management.rs` 中的 TODO 标记
- 运行 `cargo check --workspace`
- 运行 `cargo test --workspace --lib`

## 依赖关系

```
Phase 1 (rtp-core)
    ↓
Phase 2 (media-core) — 依赖 Phase 1 的新 API
    ↓
Phase 3 (session-core) — 依赖 Phase 1 + 2 的新 API
    ↓
Phase 4 (清理 + 验证) — 依赖全部完成
```

## 风险评估

| 风险 | 缓解措施 |
|------|---------|
| SecurityRtpTransport 停止内部 receiver 后重启失败 | 已有逻辑处理：`stop_receiver()` + 自启接管 |
| DTLS 握手需要 socket 但 SecurityRtpTransport 已接管 | 通过 `inner_transport().get_socket()` 获取 |
| RTP 会话在 SRTP context 安装前就开始收发 | SecurityRtpTransport 会 drop 无 context 时的包，安全 |
| 向后兼容：非 SRTP 通话不受影响 | `start_media()` 保持不变，只有 SRTP 场景走新路径 |
