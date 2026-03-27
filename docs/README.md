# rvoip Documentation

## Structure

```
docs/
├── architecture/       Design documents for core subsystems
│   ├── call-center-state-machine.md
│   ├── event-condition-matrix.md
│   ├── registrar.md
│   └── session-state-tables.md
│
├── guides/             Developer guides and API documentation
│   ├── session-api-design.md
│   ├── session-api-guide.md
│   ├── session-cookbook.md
│   ├── sip-core-developer-guide.md
│   └── sip-proxy-guide.md          — SIP proxy features: digest auth, NAT, Record-Route, forwarding
│
├── audit/              Production readiness audit reports
│   ├── 001-production-readiness-plan.md
│   ├── 002-production-audit-report.md
│   ├── 003-cross-audit-report.md
│   ├── 004-implementation-plan-sctp-ice-turn-forking.md
│   ├── 005-codex-plan-audit.md
│   ├── 006-test-plan.md
│   └── 007-commit-audit.md         — Git commit security audit (secrets and sensitive file inventory)
│
└── rfcs/               (Future) RFC compliance notes
```

## Testing

Run the full test suite:
```bash
./scripts/test_all.sh          # All levels
./scripts/test_all.sh unit     # Unit tests only (fastest)
./scripts/test_all.sh adapter  # Adapter roundtrip tests
./scripts/test_all.sh integration  # Cross-module integration
./scripts/test_all.sh e2e      # End-to-end tests
```

## Crate READMEs

Each crate has its own `README.md` with crate-specific documentation:
- [sip-core](../crates/sip-core/README.md) — SIP protocol foundation
- [rtp-core](../crates/rtp-core/README.md) — RTP/RTCP/SRTP/DTLS/ICE/STUN/TURN
- [dialog-core](../crates/dialog-core/README.md) — SIP dialog state machine
- [session-core](../crates/session-core/README.md) — Session management hub
- [media-core](../crates/media-core/README.md) — Audio processing
- [client-core](../crates/client-core/README.md) — High-level client API
- [call-engine](../crates/call-engine/README.md) — Call center orchestration
