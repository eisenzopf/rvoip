# Git Commit Security Audit

**Document ID**: AUDIT-007
**Date**: 2026-03-27
**Scope**: entire rvoip workspace

---

## Summary

| Category | Files | Status |
|----------|-------|--------|
| Hardcoded secrets / plaintext passwords | 2 (fixed) | Fixed in this audit |
| Dev defaults with env-var protection | 1 | Acceptable for dev repo only |
| New code added 2026-03-27 | All | No secrets |
| .gitignore coverage | — | Adequate after additions below |

---

## Files That Must Not Be Committed to a Public / Production Repository

### 1. `Containerfile` — **Fixed**

**Problem (before fix):** `ENV` instructions baked credentials into every image layer.
Any `docker history` or `docker inspect` on a derived image would expose them.

```dockerfile
# BEFORE (removed)
ENV DATABASE_URL=postgres://rvoip:rvoip_dev@postgres:5432/rvoip
ENV RVOIP_JWT_SECRET=change-me-in-production
ENV RVOIP_ADMIN_PASSWORD=Rvoip@Console2026!
```

**After fix:** The `ENV` lines have been replaced with a comment block.
All secrets must be supplied at `docker run` time via `-e` flags or a
`docker-compose.yml` that reads from the host environment.

```dockerfile
# AFTER
# Required at runtime — supply via -e or docker-compose environment:
#   DATABASE_URL        postgres://user:pass@host:5432/db
#   RVOIP_JWT_SECRET    cryptographically random string (32+ bytes)
#   RVOIP_ADMIN_PASSWORD  strong password for the default super-admin
#   SIP_REALM           SIP digest realm (default: rvoip)
CMD ["rvoip-console"]
```

---

### 2. `docker-compose.yml` — **Fixed**

**Problem (before fix):** PostgreSQL password was a literal string; `DATABASE_URL`
contained embedded credentials with no override mechanism.

```yaml
# BEFORE (removed)
POSTGRES_PASSWORD: rvoip_dev
DATABASE_URL: postgres://rvoip:rvoip_dev@postgres:5432/rvoip
RVOIP_JWT_SECRET: ${RVOIP_JWT_SECRET:-rvoip-production-secret-change-me}
RVOIP_ADMIN_PASSWORD: ${RVOIP_ADMIN_PASSWORD:-Rvoip@Console2026!}
```

**After fix:** All sensitive values use `${VAR:?error}` syntax, which makes
`docker-compose up` **fail fast** with a clear error if the variable is not
set in the calling environment.

```yaml
# AFTER
POSTGRES_PASSWORD: ${POSTGRES_PASSWORD:?POSTGRES_PASSWORD must be set}
DATABASE_URL: ${DATABASE_URL:?DATABASE_URL must be set}
RVOIP_JWT_SECRET: ${RVOIP_JWT_SECRET:?RVOIP_JWT_SECRET must be set}
RVOIP_ADMIN_PASSWORD: ${RVOIP_ADMIN_PASSWORD:?RVOIP_ADMIN_PASSWORD must be set}
```

Supply values via a `.env.local` file (already in `.gitignore`):

```bash
# .env.local  (never commit this file)
POSTGRES_PASSWORD=my-local-dev-password
DATABASE_URL=postgres://rvoip:my-local-dev-password@localhost:5432/rvoip
RVOIP_JWT_SECRET=dev-jwt-secret-not-for-production
RVOIP_ADMIN_PASSWORD=dev-admin-password
```

---

### 3. `crates/web-console/examples/server.rs` — Acceptable

**Assessment:** This is example / demo code. Hardcoded development defaults
(`postgres://rvoip:rvoip_dev@localhost:5432/rvoip`, `rvoip-dev-secret-change-me-in-production`)
are normal for an example binary. The file's doc comment already states it
requires local PostgreSQL.

**Condition for committing:** The file must not be used as the basis for a
production deployment without replacing all default values via environment
variables.

---

## .gitignore Status

The following entries were added to `.gitignore` in this session:

```gitignore
# Local secret overrides — see docs/audit/007-commit-audit.md
.env.local
.env.production
.env.staging
docker-compose.override.yml
Containerfile.local
secrets/
```

Previously covered (already present):

```gitignore
.env*        # all .env files
certs/       # TLS certificates
*.crt *.key *.pem
*.db         # SQLite databases
.claude/     # Claude config (may contain API keys)
*.log
target/
```

---

## Recommended Secret Management

### Development

```bash
# Create .env.local (already in .gitignore)
cat > .env.local << 'EOF'
POSTGRES_PASSWORD=dev-only-password
DATABASE_URL=postgres://rvoip:dev-only-password@localhost:5432/rvoip
RVOIP_JWT_SECRET=dev-jwt-not-for-production
RVOIP_ADMIN_PASSWORD=dev-admin
SIP_REALM=dev.local
EOF

source .env.local
cargo run -p rvoip-web-console --example web_console_server
```

### Production

Use one of the following — never hardcode in source:

```bash
# Option A: system environment
export RVOIP_JWT_SECRET=$(openssl rand -hex 32)
export POSTGRES_PASSWORD=$(openssl rand -hex 16)

# Option B: Docker secrets
echo "$(openssl rand -hex 32)" | docker secret create rvoip_jwt_secret -

# Option C: Kubernetes Secrets / AWS Secrets Manager / Vault
```

---

## Security Audit of Code Added 2026-03-27

| File | Contains secrets? |
|------|-------------------|
| `crates/web-console/src/sip_providers.rs` | No — DB connection injected via `Arc<DatabaseManager>` |
| `crates/dialog-core/src/auth.rs` | No — trait definitions and noop stub only |
| `crates/dialog-core/src/manager/core.rs` | No — RwLock wrappers and setters only |
| `crates/dialog-core/src/api/unified.rs` | No — delegation methods only |
| `crates/session-core/src/dialog/manager.rs` | No — delegation methods only |
| `crates/session-core/src/coordinator/coordinator.rs` | No |
| `crates/call-engine/src/orchestrator/core.rs` | No |
| All other dialog-core / session-core changes | No |

All new code is safe to commit.

---

## Action Items

| Priority | File | Action |
|----------|------|--------|
| Done | `Containerfile` | Removed hardcoded `ENV` secrets |
| Done | `docker-compose.yml` | Changed to `${VAR:?}` mandatory substitution |
| Done | `.gitignore` | Added local override patterns |
| Ongoing | Any new config files | Must never contain literal secrets |
