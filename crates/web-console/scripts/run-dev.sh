#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

echo "=== Starting PostgreSQL ==="
podman start rvoip-postgres 2>/dev/null || \
podman run -d --name rvoip-postgres \
  -e POSTGRES_USER=rvoip -e POSTGRES_PASSWORD=rvoip_dev -e POSTGRES_DB=rvoip \
  -p 5432:5432 -v rvoip-pgdata:/var/lib/postgresql/data \
  docker.io/library/postgres:18-alpine

echo "Waiting for PostgreSQL..."
sleep 3

echo "=== Building frontend ==="
cd frontend
npm ci --silent
npm run build
cd ..

echo "=== Starting rvoip console ==="
cd ../..
cargo run -p rvoip-web-console --example web_console_server
