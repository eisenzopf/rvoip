# Stage 1: Build frontend
FROM node:22-alpine AS frontend
WORKDIR /app/frontend
COPY crates/web-console/frontend/package.json crates/web-console/frontend/package-lock.json* ./
RUN npm ci
COPY crates/web-console/frontend/ ./
RUN npm run build

# Stage 2: Build Rust backend
FROM rust:1.85-bookworm AS backend
WORKDIR /app
# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
# Copy frontend dist into the web-console crate for rust-embed
COPY --from=frontend /app/frontend/dist crates/web-console/frontend/dist/
# Build release binary
RUN cargo build --release -p rvoip-web-console --example web_console_server

# Stage 3: Runtime
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=backend /app/target/release/examples/web_console_server /usr/local/bin/rvoip-console
EXPOSE 3000 5060/udp 8080
# Required at runtime — supply via -e or docker-compose environment:
#   DATABASE_URL        postgres://user:pass@host:5432/db
#   RVOIP_JWT_SECRET    cryptographically random string (32+ bytes)
#   RVOIP_ADMIN_PASSWORD  strong password for the default super-admin account
#   SIP_REALM           SIP digest realm (default: rvoip)
CMD ["rvoip-console"]
