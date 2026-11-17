---
section: Breaking changes
breaking: true
---
**redis 0.27 → 1.x in `rvoip-redis`.** `RedisAuthError::Redis` wraps a
redis 1.x `RedisError`. Migration: depend on redis 1 when you match on
the wrapped error's kind (`ErrorKind::IoError` is now `ErrorKind::Io`).
