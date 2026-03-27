# rvoip Web Console

Web-based management console for the rvoip SIP/VoIP stack.

## Quick Start

### Prerequisites

- Rust 1.85+ (Edition 2024)
- Node.js 22+
- PostgreSQL 18 (via Podman/Docker)

### 1. Start PostgreSQL

```bash
podman run -d --name rvoip-postgres \
  -e POSTGRES_USER=rvoip \
  -e POSTGRES_PASSWORD=rvoip_dev \
  -e POSTGRES_DB=rvoip \
  -p 5432:5432 \
  -v rvoip-pgdata:/var/lib/postgresql/data \
  docker.io/library/postgres:18-alpine
```

### 2. Build & Run

```bash
# Build frontend
cd crates/web-console/frontend
npm ci && npm run build
cd ../../..

# Run server
cargo run -p rvoip-web-console --example web_console_server
```

### 3. Open Console

```
URL:      http://127.0.0.1:3000
Username: admin
Password: Rvoip@Console2026!
```

The default admin account is created automatically on first startup.

## Container Deployment

```bash
# Build and start everything
podman-compose up -d

# Or with docker-compose
docker compose up -d
```

Services:
- `console` — rvoip web console on port 3000
- `postgres` — PostgreSQL 18 on port 5432

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | `postgres://rvoip:rvoip_dev@localhost:5432/rvoip` | PostgreSQL connection string |
| `RVOIP_JWT_SECRET` | dev default | JWT signing secret (HS256) |
| `RVOIP_ADMIN_PASSWORD` | `Rvoip@Console2026!` | Initial admin password |
| `RUST_LOG` | `info` | Log level (`debug` for verbose) |

## Features

### Pages (15)

| Page | Route | Access |
|------|-------|--------|
| Login | `/login` | Public |
| Dashboard | `/` | All roles |
| Active Calls | `/calls` | Supervisor+ |
| Call History | `/calls/history` | Supervisor+ |
| Agents (CRUD) | `/agents` | Admin+ manage |
| Queues (CRUD) | `/queues` | Admin+ manage |
| Routing Config | `/routing` | Admin+ |
| SIP Registrations | `/registrations` | Supervisor+ |
| Presence | `/presence` | All roles |
| Users (CRUD) | `/users` | Admin+ |
| Monitoring | `/monitoring` | Supervisor+ |
| System Config | `/system/config` | Admin+ |
| Audit Log | `/system/audit` | Admin+ |
| Health | `/health` | All roles |
| Profile | `/profile` | All roles |
| API Keys | `/profile/api-keys` | Supervisor+ |

### RBAC Roles

| Role | Level | Description |
|------|-------|-------------|
| `super_admin` | Full | All permissions, role assignment |
| `admin` | Management | User/agent/queue/routing CRUD |
| `supervisor` | Monitoring | View all + assign calls + coaching |
| `agent` | Self-service | Own calls, status, profile only |

### Security

- JWT authentication (HS256, 15min access / 30d refresh tokens)
- RBAC permission middleware on all API routes
- Rate limiting (200 req/min API, 5 req/min login)
- Security headers (CSP, X-Frame-Options, HSTS)
- Audit logging (all write operations to PostgreSQL)
- Optional TLS/HTTPS (feature flag `tls`)

### API Endpoints (30+)

```
Auth:        POST login/logout/refresh, GET me, PUT password
Users:       GET/POST/PUT/DELETE /users, PUT roles, API keys
Agents:      GET/POST/PUT/DELETE /agents
Queues:      GET/POST/PUT/DELETE /queues, queue calls, assign
Routing:     GET/PUT config, CRUD overflow policies
Calls:       GET active/history, POST hangup
Registrations: GET list
Presence:    GET all/user, PUT me, GET buddies
System:      GET health/config, POST export/import, audit log
Monitoring:  GET realtime/alerts/events
Dashboard:   GET metrics/activity
WebSocket:   WS /ws/events (real-time)
```

### Tech Stack

- **Backend**: Rust + Axum 0.8 + SQLx + PostgreSQL 18
- **Frontend**: React 19 + TypeScript + Vite + shadcn/ui + Tailwind CSS
- **i18n**: Chinese + English (react-i18next)
- **Themes**: Light / Dark / System
- **Embedded**: Frontend compiled into Rust binary via rust-embed

## Development

```bash
# Frontend dev server (hot reload, proxies API to :3000)
cd crates/web-console/frontend
npm run dev    # → http://localhost:5173

# Backend dev (in another terminal)
cargo run -p rvoip-web-console --example web_console_server

# Run integration tests
bash crates/web-console/scripts/integration-test.sh

# Enable TLS
cargo run -p rvoip-web-console --features tls --example web_console_server
```

## Database Schema

Tables managed by the web console:

| Table | Source | Purpose |
|-------|--------|---------|
| `agents` | call-engine | SIP agents |
| `call_queue` | call-engine | Queued calls |
| `active_calls` | call-engine | Current calls |
| `queues` | call-engine | Queue configuration |
| `call_records` | call-engine | Call history |
| `users` | users-core | User accounts |
| `api_keys` | users-core | API keys |
| `refresh_tokens` | users-core | JWT refresh tokens |
| `sessions` | users-core | User sessions |
| `overflow_policies` | web-console | Overflow routing rules |
| `audit_log` | web-console | Operation audit trail |
