# rvoip Web Console 全功能管理平台实施计划

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 将 rvoip web console 从展示型仪表盘升级为可管理 rvoip 所有模块、所有实体的全功能运营平台，包含 RBAC 权限体系和完整业务流程。

**Architecture:** 基于现有 Axum 后端 + React 前端，新增认证层（JWT）、权限中间件、完整 CRUD API、配置管理、操作审计日志。后端拆分为 auth middleware → RBAC guard → business handler 三层。前端按模块拆分，每个模块独立路由+懒加载。

**Tech Stack:** Rust/Axum 0.8 + PostgreSQL 18 + SQLx | React 19 + TypeScript + shadcn/ui v4 + react-i18next + react-router + @tanstack/react-query

---

## 一、权限体系设计

### 1.1 角色模型

| 角色 | 权限级别 | 说明 |
|------|----------|------|
| `super_admin` | 全部权限 | 系统超级管理员，第一个注册的用户自动获得 |
| `admin` | 管理权限 | 可管理坐席、队列、路由、配置，不可管理其他管理员 |
| `supervisor` | 监控+有限管理 | 可查看所有数据、强制分配通话、教练坐席，不可改配置 |
| `agent` | 坐席权限 | 只能看自己的通话、状态、绩效，不可看其他坐席数据 |

### 1.2 权限矩阵

| 资源 | super_admin | admin | supervisor | agent |
|------|:-----------:|:-----:|:----------:|:-----:|
| **用户管理** |
| 创建/删除用户 | ✅ | ✅ | ❌ | ❌ |
| 分配角色 | ✅ | ❌ | ❌ | ❌ |
| 修改自己密码 | ✅ | ✅ | ✅ | ✅ |
| **坐席管理** |
| 创建/删除坐席 | ✅ | ✅ | ❌ | ❌ |
| 修改坐席属性 | ✅ | ✅ | ✅ | ❌ |
| 查看所有坐席 | ✅ | ✅ | ✅ | ❌ |
| 修改自己状态 | ✅ | ✅ | ✅ | ✅ |
| **队列管理** |
| 创建/删除队列 | ✅ | ✅ | ❌ | ❌ |
| 修改队列配置 | ✅ | ✅ | ❌ | ❌ |
| 查看队列状态 | ✅ | ✅ | ✅ | ✅(仅分配的) |
| 手动分配通话 | ✅ | ✅ | ✅ | ❌ |
| **路由配置** |
| 修改路由策略 | ✅ | ✅ | ❌ | ❌ |
| 修改溢出策略 | ✅ | ✅ | ❌ | ❌ |
| 查看路由规则 | ✅ | ✅ | ✅ | ❌ |
| **系统配置** |
| 修改全局配置 | ✅ | ❌ | ❌ | ❌ |
| 导入/导出配置 | ✅ | ✅ | ❌ | ❌ |
| 数据库维护 | ✅ | ❌ | ❌ | ❌ |
| **通话管理** |
| 查看所有通话 | ✅ | ✅ | ✅ | ❌ |
| 查看自己通话 | ✅ | ✅ | ✅ | ✅ |
| 监听/插话/教练 | ✅ | ✅ | ✅ | ❌ |
| 强制挂断 | ✅ | ✅ | ✅ | ❌ |
| **SIP注册** |
| 查看注册列表 | ✅ | ✅ | ✅ | ❌ |
| 强制注销 | ✅ | ✅ | ❌ | ❌ |
| 修改注册配置 | ✅ | ✅ | ❌ | ❌ |
| **监控与报表** |
| 查看实时仪表盘 | ✅ | ✅ | ✅ | ✅(有限) |
| 查看历史报表 | ✅ | ✅ | ✅ | ❌ |
| 导出数据 | ✅ | ✅ | ✅ | ❌ |
| 查看审计日志 | ✅ | ✅ | ❌ | ❌ |
| **Presence** |
| 查看在线状态 | ✅ | ✅ | ✅ | ✅(好友) |
| 修改自己状态 | ✅ | ✅ | ✅ | ✅ |
| 管理好友列表 | ✅ | ✅ | ✅ | ✅ |
| **API Key** |
| 创建自己的 Key | ✅ | ✅ | ✅ | ❌ |
| 管理所有 Key | ✅ | ❌ | ❌ | ❌ |

### 1.3 默认管理员策略

- 系统首次启动时，自动创建 `admin` 用户 (密码从环境变量 `RVOIP_ADMIN_PASSWORD` 读取，默认 `admin123`)
- 该用户自动获得 `super_admin` 角色
- 后续用户由 `super_admin` 或 `admin` 创建并分配角色
- 每个用户可同时拥有坐席身份（关联 agent_id）

---

## 二、模块规划总览

### 后端 API 端点设计 (`/api/v1/`)

```
认证 Auth
├── POST   /auth/login                    # 登录 → JWT
├── POST   /auth/logout                   # 登出（废弃 refresh token）
├── POST   /auth/refresh                  # 刷新 token
├── GET    /auth/me                       # 当前用户信息
└── PUT    /auth/me/password              # 修改自己密码

用户管理 Users (admin+)
├── GET    /users                         # 列表（分页/搜索/角色筛选）
├── POST   /users                         # 创建用户
├── GET    /users/:id                     # 详情
├── PUT    /users/:id                     # 更新
├── DELETE /users/:id                     # 删除
├── PUT    /users/:id/roles               # 分配角色 (super_admin only)
├── PUT    /users/:id/password            # 重置密码
├── GET    /users/:id/api-keys            # 该用户的 API Key 列表
├── POST   /users/:id/api-keys            # 创建 API Key
└── DELETE /users/:id/api-keys/:key_id    # 吊销 API Key

坐席管理 Agents (admin+ 管理, agent 查看自己)
├── GET    /agents                        # 列表
├── POST   /agents                        # 创建坐席
├── GET    /agents/:id                    # 详情
├── PUT    /agents/:id                    # 更新（SIP URI/技能/最大并发/部门）
├── DELETE /agents/:id                    # 删除
├── PUT    /agents/:id/status             # 修改状态 (Available/Busy/Offline)
├── PUT    /agents/:id/skills             # 修改技能列表
└── GET    /agents/:id/stats              # 坐席绩效统计

队列管理 Queues (admin+ 管理, supervisor+ 查看)
├── GET    /queues                        # 列表（含实时统计）
├── POST   /queues                        # 创建队列
├── GET    /queues/:id                    # 详情 + 排队中的通话
├── PUT    /queues/:id                    # 更新配置
├── DELETE /queues/:id                    # 删除
├── GET    /queues/:id/calls              # 队列中的通话列表
└── POST   /queues/:id/calls/:call_id/assign  # 手动分配通话给坐席

路由配置 Routing (admin+)
├── GET    /routing/config                # 当前路由配置
├── PUT    /routing/config                # 更新路由策略
├── GET    /routing/overflow              # 溢出策略列表
├── POST   /routing/overflow/policies     # 添加溢出策略
├── PUT    /routing/overflow/policies/:id # 更新溢出策略
├── DELETE /routing/overflow/policies/:id # 删除溢出策略
├── GET    /routing/overflow/stats        # 溢出统计
└── GET    /routing/skills                # 技能列表及层级

通话管理 Calls (supervisor+ 全部, agent 仅自己)
├── GET    /calls                         # 活跃通话列表
├── GET    /calls/:id                     # 通话详情
├── POST   /calls/:id/hold               # 保持
├── POST   /calls/:id/transfer/:target    # 转接
├── POST   /calls/:id/hangup             # 挂断
├── GET    /calls/history                 # 历史通话记录（分页）
└── GET    /calls/history/:id             # 历史通话详情

SIP 注册 Registrations (supervisor+)
├── GET    /registrations                 # 列表
├── GET    /registrations/:user_id        # 详情（含所有 contact binding）
├── DELETE /registrations/:user_id        # 强制注销
└── GET    /registrations/config          # 注册配置

Presence 在线状态 (全部角色)
├── GET    /presence                      # 所有在线用户状态
├── GET    /presence/:user_id             # 指定用户状态
├── PUT    /presence/me                   # 修改自己状态
├── GET    /presence/buddies              # 我的好友列表
└── POST   /presence/subscribe/:user_id   # 订阅某人状态

系统配置 System (admin+ 查看, super_admin 修改)
├── GET    /system/health                 # 健康检查
├── GET    /system/config                 # 当前系统配置
├── PUT    /system/config                 # 修改配置 (super_admin)
├── POST   /system/config/export          # 导出配置 JSON
├── POST   /system/config/import          # 导入配置 JSON
├── GET    /system/stats                  # 系统统计
├── POST   /system/db/optimize            # 数据库优化 (super_admin)
└── GET    /system/audit-log              # 操作审计日志（分页）

仪表盘 Dashboard (全部角色, 数据范围受限)
├── GET    /dashboard                     # 聚合指标
├── GET    /dashboard/activity            # 24h 通话趋势
└── GET    /dashboard/agent/:id           # 坐席个人仪表盘 (agent)

监控 Monitoring (supervisor+)
├── GET    /monitoring/realtime           # 实时统计
├── GET    /monitoring/alerts             # 活跃告警
├── GET    /monitoring/performance        # 绩效摘要
├── POST   /monitoring/report/daily       # 生成日报
├── POST   /monitoring/report/custom      # 自定义报表
└── GET    /monitoring/events             # 历史事件查询

WebSocket (全部角色, 事件过滤按权限)
└── WS     /ws/events                     # 实时事件流
```

---

## 三、前端页面规划

### 3.1 页面路由结构

```
/login                          # 登录页（无需认证）
/                               # Dashboard（按角色展示不同内容）
/calls                          # 活跃通话（supervisor+ 全部，agent 仅自己）
/calls/history                  # 历史通话记录
/agents                         # 坐席管理（admin+ CRUD，supervisor 只读+操作）
/agents/:id                     # 坐席详情（绩效、通话历史）
/queues                         # 队列管理（admin+ CRUD，supervisor 查看+分配）
/queues/:id                     # 队列详情（排队中的通话、配置）
/routing                        # 路由配置（admin+）
/routing/overflow               # 溢出策略管理
/registrations                  # SIP 注册列表（supervisor+）
/presence                       # 在线状态总览
/users                          # 用户管理（admin+）
/users/:id                      # 用户详情
/system/config                  # 系统配置（admin+）
/system/audit                   # 审计日志（admin+）
/monitoring                     # 监控中心（supervisor+）
/monitoring/reports             # 报表生成
/profile                        # 个人设置（所有角色）
/profile/api-keys               # 我的 API Key
```

### 3.2 按角色的侧边栏菜单

**super_admin / admin 看到全部菜单**

**supervisor 看到：**
- 仪表盘、活跃通话、通话历史、坐席(只读+操作)、队列(查看+分配)、
  路由(只读)、SIP 注册、在线状态、监控中心、报表

**agent 看到：**
- 我的仪表盘、我的通话、在线状态、个人设置

### 3.3 新增组件

| 组件 | 用途 |
|------|------|
| `LoginPage` | 登录表单，JWT 获取 |
| `AuthGuard` | 路由守卫，检查 token + 角色 |
| `ProtectedRoute` | 基于角色的路由包装 |
| `UsersCrud` | 用户 CRUD 表格 + 创建/编辑 Dialog |
| `RoleAssigner` | 角色分配下拉 |
| `QueueEditor` | 队列创建/编辑表单 |
| `QueueDetail` | 队列详情 + 排队通话列表 + 手动分配 |
| `RoutingConfig` | 路由策略选择器 + 负载均衡策略 |
| `OverflowPolicies` | 溢出策略 CRUD（条件+动作组合） |
| `CallHistory` | 历史通话 DataTable（分页/搜索/导出） |
| `AgentDetail` | 坐席详情页（绩效图表、通话历史） |
| `SystemConfig` | 全局配置编辑器（分 section 展示） |
| `AuditLog` | 审计日志查看器（时间范围筛选） |
| `MonitoringCenter` | 实时监控大屏（告警、绩效、队列热力图） |
| `ReportGenerator` | 报表生成器（日报/周报/自定义） |
| `PresenceBoard` | 在线状态看板 |
| `ProfilePage` | 个人设置（密码、偏好、API Key） |
| `ApiKeyManager` | API Key 创建/列表/吊销 |

---

## 四、实施分期

### Phase 4A — 认证与权限基础 (核心，优先级最高)

| 任务 | 说明 |
|------|------|
| 4A-1 | 后端：集成 users-core 到 web-console，JWT 认证中间件 |
| 4A-2 | 后端：RBAC 权限中间件（从 JWT claims 提取角色，按路由验证） |
| 4A-3 | 后端：`/auth/login`、`/auth/logout`、`/auth/refresh`、`/auth/me` |
| 4A-4 | 后端：首次启动自动创建 super_admin 用户 |
| 4A-5 | 前端：Login 页面 + JWT 存储（httpOnly cookie 或 localStorage） |
| 4A-6 | 前端：AuthGuard + ProtectedRoute + 角色感知侧边栏 |
| 4A-7 | 前端：个人设置页 (修改密码) |

### Phase 4B — 用户管理 CRUD

| 任务 | 说明 |
|------|------|
| 4B-1 | 后端：`/users` 完整 CRUD（集成 users-core UserStore） |
| 4B-2 | 后端：`/users/:id/roles` 角色分配端点 |
| 4B-3 | 后端：`/users/:id/api-keys` API Key 管理端点 |
| 4B-4 | 前端：用户管理页面（DataTable + 创建/编辑/删除 Dialog） |
| 4B-5 | 前端：角色分配 UI（Select 组件） |
| 4B-6 | 前端：API Key 管理页面 |

### Phase 4C — 队列完整管理

| 任务 | 说明 |
|------|------|
| 4C-1 | 后端：`/queues` CRUD（调用 AdminApi create/update/delete_queue） |
| 4C-2 | 后端：`/queues/:id/calls` 列出排队通话 + 手动分配 |
| 4C-3 | 前端：队列 CRUD 页面（创建/编辑表单含优先级、最大等待、溢出开关） |
| 4C-4 | 前端：队列详情页（排队通话列表 + 手动分配给坐席 Dialog） |

### Phase 4D — 路由与溢出策略

| 任务 | 说明 |
|------|------|
| 4D-1 | 后端：`/routing/config` GET/PUT（策略选择、负载均衡） |
| 4D-2 | 后端：`/routing/overflow/*` CRUD（溢出策略：条件+动作） |
| 4D-3 | 前端：路由配置页（RoutingStrategy 单选 + LoadBalanceStrategy + 开关） |
| 4D-4 | 前端：溢出策略管理（策略列表 + 新建/编辑 Dialog，条件选择器+动作选择器） |

### Phase 4E — 通话操作与历史

| 任务 | 说明 |
|------|------|
| 4E-1 | 后端：通话操作端点（hold/transfer/hangup）调用 session-core |
| 4E-2 | 后端：`/calls/history` 查询（从 call_records 表分页读取） |
| 4E-3 | 前端：通话操作按钮（保持/转接/挂断）添加到 Calls 页面 |
| 4E-4 | 前端：通话历史页面（DataTable + 时间范围选择器 + 导出 CSV） |

### Phase 4F — 系统配置管理

| 任务 | 说明 |
|------|------|
| 4F-1 | 后端：`/system/config` GET/PUT 全局配置（GeneralConfig + 各子配置） |
| 4F-2 | 后端：`/system/config/export`、`/system/config/import` |
| 4F-3 | 后端：`/system/audit-log` 审计日志（记录所有管理操作到新表） |
| 4F-4 | 后端：`/system/db/optimize` 数据库优化 |
| 4F-5 | 前端：系统配置页面（按 section 分 Tab：通用/坐席/队列/路由/监控/数据库） |
| 4F-6 | 前端：配置导入导出按钮 |
| 4F-7 | 前端：审计日志页面（时间筛选 + 操作类型筛选 + 用户筛选） |

### Phase 4G — Presence 与注册增强

| 任务 | 说明 |
|------|------|
| 4G-1 | 后端：`/presence/*` 端点（查看/修改/订阅） |
| 4G-2 | 后端：`/registrations/:user_id` 注册详情 + 强制注销 |
| 4G-3 | 后端：`/registrations/config` 注册配置读写 |
| 4G-4 | 前端：Presence 看板（在线用户网格，状态图标，活动指示） |
| 4G-5 | 前端：注册详情页（Contact 列表、强制注销按钮、注册配置编辑） |

### Phase 4H — 监控中心与报表

| 任务 | 说明 |
|------|------|
| 4H-1 | 后端：`/monitoring/*` 端点（实时统计、告警、绩效、报表） |
| 4H-2 | 前端：监控中心大屏（实时数据卡片、告警列表、队列热力图） |
| 4H-3 | 前端：报表生成页面（选择类型、时间范围、预览、下载） |
| 4H-4 | 前端：坐席详情页优化（绩效历史折线图、通话分布饼图） |

---

## 五、数据库新增表

```sql
-- 审计日志表
CREATE TABLE IF NOT EXISTS audit_log (
    id BIGSERIAL PRIMARY KEY,
    user_id TEXT NOT NULL,
    username TEXT NOT NULL,
    action TEXT NOT NULL,            -- CREATE_AGENT, DELETE_QUEUE, UPDATE_CONFIG, ...
    resource_type TEXT NOT NULL,     -- agent, queue, user, config, ...
    resource_id TEXT,
    details JSONB,                   -- 操作详情（旧值/新值）
    ip_address TEXT,
    user_agent TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_audit_log_user ON audit_log(user_id);
CREATE INDEX idx_audit_log_action ON audit_log(action);
CREATE INDEX idx_audit_log_resource ON audit_log(resource_type, resource_id);
CREATE INDEX idx_audit_log_time ON audit_log(created_at);

-- 溢出策略持久化表
CREATE TABLE IF NOT EXISTS overflow_policies (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    condition_type TEXT NOT NULL,
    condition_value TEXT NOT NULL,
    action_type TEXT NOT NULL,
    action_value TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 5,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 通话历史补充索引
CREATE INDEX IF NOT EXISTS idx_call_records_time_range ON call_records(start_time, end_time);
CREATE INDEX IF NOT EXISTS idx_call_records_disposition ON call_records(disposition);
```

---

## 六、关键设计决策

### 6.1 认证流程

```
前端 Login → POST /auth/login (username + password)
         ← { access_token (15min), refresh_token (7d) }

前端请求 → Authorization: Bearer <access_token>
         → 后端 JWT 验证中间件提取 { user_id, username, roles }
         → RBAC 中间件检查角色权限
         → Handler 处理请求

Token 过期 → 前端自动 POST /auth/refresh → 新 token pair
```

### 6.2 用户与坐席关联

- `users` 表存账号信息（登录用）
- `agents` 表存坐席信息（SIP/呼叫用）
- 通过 `users.id` → `agents.agent_id` 关联（非外键，允许坐席不绑定用户）
- 创建坐席时可选择关联用户，或独立存在
- admin 创建用户时可同时创建关联坐席

### 6.3 审计日志策略

- 所有 **写操作**（POST/PUT/DELETE）自动记录审计日志
- 通过 Axum middleware 统一拦截，无需每个 handler 手动记录
- 日志包含：操作人、操作类型、资源类型/ID、请求体摘要、IP、时间
- 保留 90 天（可配置）

### 6.4 WebSocket 权限过滤

- 连接时验证 JWT（通过 query parameter `?token=xxx`）
- 根据角色过滤推送事件：
  - `agent`: 只收到自己相关的通话事件
  - `supervisor`: 收到所有通话+坐席+队列事件
  - `admin`/`super_admin`: 收到所有事件 + 系统事件

---

## 七、实施优先级建议

```
Phase 4A (认证权限) ──→ Phase 4B (用户管理) ──→ Phase 4C (队列CRUD)
                                               ↘
                                                Phase 4D (路由配置)
                                               ↗
Phase 4E (通话操作) ──→ Phase 4F (系统配置) ──→ Phase 4G (Presence)
                                               ↘
                                                Phase 4H (监控报表)
```

**建议执行顺序：4A → 4B → 4C → 4D → 4E → 4F → 4G → 4H**

Phase 4A 是所有后续功能的前置条件。4B-4D 可并行。4E-4H 可并行。

---

## 八、工作量估算

| Phase | 后端工作量 | 前端工作量 | 总计 |
|-------|-----------|-----------|------|
| 4A 认证权限 | 重 | 重 | ⭐⭐⭐⭐ |
| 4B 用户管理 | 中 | 中 | ⭐⭐⭐ |
| 4C 队列管理 | 中 | 中 | ⭐⭐⭐ |
| 4D 路由配置 | 轻 | 中 | ⭐⭐ |
| 4E 通话操作 | 重 | 中 | ⭐⭐⭐ |
| 4F 系统配置 | 中 | 重 | ⭐⭐⭐ |
| 4G Presence | 中 | 中 | ⭐⭐⭐ |
| 4H 监控报表 | 中 | 重 | ⭐⭐⭐⭐ |
