#!/usr/bin/env bash
# rvoip web console integration test
# Prerequisites: PostgreSQL running on localhost:5432, server binary built
set -euo pipefail

BASE="http://127.0.0.1:3000/api/v1"
PASS=0
FAIL=0

ok()   { PASS=$((PASS+1)); echo "  ✅ $1"; }
fail() { FAIL=$((FAIL+1)); echo "  ❌ $1: $2"; }

http() {
  local method=$1 path=$2; shift 2
  curl -sf -X "$method" "${BASE}${path}" \
    -H "Authorization: Bearer ${TOKEN:-}" \
    -H 'Content-Type: application/json' \
    "$@" 2>/dev/null
}

http_status() {
  local method=$1 path=$2; shift 2
  curl -s -o /dev/null -w "%{http_code}" -X "$method" "${BASE}${path}" \
    -H "Authorization: Bearer ${TOKEN:-}" \
    -H 'Content-Type: application/json' \
    "$@" 2>/dev/null
}

echo "═══════════════════════════════════════"
echo "  rvoip Web Console Integration Tests"
echo "═══════════════════════════════════════"
echo ""

# ── Auth ─────────────────────────────────────────────────────────────────────
echo "▸ Authentication"

STATUS=$(http_status POST /auth/login --data-raw '{"username":"admin","password":"Rvoip@Console2026!"}')
if [ "$STATUS" = "200" ]; then ok "login → 200"; else fail "login" "$STATUS"; fi

RESP=$(http POST /auth/login --data-raw '{"username":"admin","password":"Rvoip@Console2026!"}')
TOKEN=$(echo "$RESP" | python3 -c "import sys,json; print(json.load(sys.stdin)['access_token'])" 2>/dev/null || echo "")
if [ -n "$TOKEN" ]; then ok "got JWT token"; else fail "token" "empty"; fi

STATUS=$(http_status GET /auth/me)
if [ "$STATUS" = "200" ]; then ok "/auth/me → 200"; else fail "/auth/me" "$STATUS"; fi

STATUS=$(http_status GET /agents)
if [ "$STATUS" = "200" ]; then ok "authed /agents → 200"; else fail "authed /agents" "$STATUS"; fi

# Without token
OLD_TOKEN=$TOKEN; TOKEN=""
STATUS=$(http_status GET /agents)
if [ "$STATUS" = "401" ]; then ok "no-token /agents → 401"; else fail "no-token" "$STATUS"; fi
TOKEN=$OLD_TOKEN

echo ""

# ── Users CRUD ───────────────────────────────────────────────────────────────
echo "▸ Users CRUD"

RESP=$(http POST /users --data-raw '{"username":"testbot","password":"TestBot@Secure2026","display_name":"Test Bot","roles":["agent"]}')
USER_ID=$(echo "$RESP" | python3 -c "import sys,json; print(json.load(sys.stdin)['data']['id'])" 2>/dev/null || echo "")
if [ -n "$USER_ID" ]; then ok "create user → $USER_ID"; else fail "create user" "no id"; fi

STATUS=$(http_status GET "/users/$USER_ID")
if [ "$STATUS" = "200" ]; then ok "get user → 200"; else fail "get user" "$STATUS"; fi

STATUS=$(http_status PUT "/users/$USER_ID" --data-raw '{"display_name":"Updated Bot"}')
if [ "$STATUS" = "200" ]; then ok "update user → 200"; else fail "update user" "$STATUS"; fi

STATUS=$(http_status DELETE "/users/$USER_ID")
if [ "$STATUS" = "200" ]; then ok "delete user → 200"; else fail "delete user" "$STATUS"; fi

echo ""

# ── Agents CRUD ──────────────────────────────────────────────────────────────
echo "▸ Agents CRUD"

STATUS=$(http_status POST /agents --data-raw '{"id":"test-agent","sip_uri":"sip:test@rvoip.local","display_name":"Test Agent","skills":["english"],"max_concurrent_calls":2}')
if [ "$STATUS" = "200" ]; then ok "create agent → 200"; else fail "create agent" "$STATUS"; fi

STATUS=$(http_status GET /agents)
if [ "$STATUS" = "200" ]; then ok "list agents → 200"; else fail "list agents" "$STATUS"; fi

STATUS=$(http_status DELETE /agents/test-agent)
if [ "$STATUS" = "200" ]; then ok "delete agent → 200"; else fail "delete agent" "$STATUS"; fi

echo ""

# ── Queues ───────────────────────────────────────────────────────────────────
echo "▸ Queues"

STATUS=$(http_status POST /queues --data-raw '{"queue_id":"test-queue"}')
if [ "$STATUS" = "200" ]; then ok "create queue → 200"; else fail "create queue" "$STATUS"; fi

STATUS=$(http_status GET /queues)
if [ "$STATUS" = "200" ]; then ok "list queues → 200"; else fail "list queues" "$STATUS"; fi

echo ""

# ── Routing ──────────────────────────────────────────────────────────────────
echo "▸ Routing"

STATUS=$(http_status GET /routing/config)
if [ "$STATUS" = "200" ]; then ok "routing config → 200"; else fail "routing config" "$STATUS"; fi

STATUS=$(http_status GET /routing/overflow/policies)
if [ "$STATUS" = "200" ]; then ok "overflow policies → 200"; else fail "overflow policies" "$STATUS"; fi

echo ""

# ── Read-only endpoints ──────────────────────────────────────────────────────
echo "▸ Read-only endpoints"

for EP in /dashboard /dashboard/activity /calls /calls/history /registrations \
          /presence /monitoring/realtime /monitoring/alerts \
          /system/health /system/config /system/audit/log; do
  STATUS=$(http_status GET "$EP")
  if [ "$STATUS" = "200" ]; then ok "$EP → 200"; else fail "$EP" "$STATUS"; fi
done

echo ""

# ── Frontend ─────────────────────────────────────────────────────────────────
echo "▸ Frontend"

STATUS=$(curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:3000/)
if [ "$STATUS" = "200" ]; then ok "/ → 200 (HTML)"; else fail "/" "$STATUS"; fi

STATUS=$(curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:3000/login)
if [ "$STATUS" = "200" ]; then ok "/login → 200 (SPA)"; else fail "/login" "$STATUS"; fi

echo ""

# ── Summary ──────────────────────────────────────────────────────────────────
echo "═══════════════════════════════════════"
TOTAL=$((PASS+FAIL))
echo "  Results: $PASS/$TOTAL passed, $FAIL failed"
echo "═══════════════════════════════════════"

if [ "$FAIL" -gt 0 ]; then exit 1; fi
