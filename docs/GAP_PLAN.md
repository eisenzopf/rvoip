# rvoip — Gap Plan (outstanding work)

**v1 surface closed 2026-05-26.** All `[V1]` gap rows and phases P1–P12 landed,
and the v2.A + v2.B architectural round shipped (carve `rvoip-core-traits` to
break the dep cycle; per-tenant `Semaphore` admission). All workspace lib tests
pass.

This document was trimmed (2026-06-01) to track **only what remains**. The full
phase-by-phase implementation history (P1–P12, v2.A/v2.B, the gap inventory, and
the original roadmap) is in git history — `git log --follow docs/GAP_PLAN.md`.

Spec references point into the sibling design docs:
[`INTERFACE_DESIGN.md`](INTERFACE_DESIGN.md), [`PRD.md`](PRD.md),
[`CONVERSATION_PROTOCOL.md`](CONVERSATION_PROTOCOL.md).

## Outstanding `[V1.x]` items

Started or partially landed — these have concrete next steps.

| # | Item | Status / next step | Spec source |
|---|---|---|---|
| 3.O.8 (follow-up) | WebRTC DTMF (RFC 4733) decode | SIP DTMF, WebRTC quality, and UCTP-family DTMF (P5/P9) are wired. **Remaining:** PT-101 frames flow through `rvoip-webrtc` `media::pump` as `MediaFrame { payload_type: Some(101) }` but no RFC 4733 decoder runs on them — needs a pump-side decoder + event channel. | CONVERSATION_PROTOCOL §7.5, §10.3 |
| 3.O.9 | Inline envelope signatures enforced at adapter boundary | JCS + verify primitives exist in `signing.rs`; the required-signed policy is **not yet gated at adapter ingress**. | CONVERSATION_PROTOCOL §5.5.1 |
| 3.O.10 | `rvoip-vcon-postgres` reference store | Crate **absent**; PRD §14.2 #8 calls for shipping it as an optional crate. | INTERFACE_DESIGN §11.5 |

## Deferred backlog (per design docs — no work proposed yet)

Tracked for visibility only. When one is taken up, add a phase to this doc.

| Item | Label | Spec source |
|---|---|---|
| AAuth production hardening (gated `aauth-experimental`; the validator already landed) | `[V1.x]` | INTERFACE_DESIGN §2.4, §8.5 |
| RFC 9421 default-on per-request signing | `[V1.x]` | INTERFACE_DESIGN §2.4 |
| DTLS-SRTP fingerprint binding default-on | `[V1.x]` | INTERFACE_DESIGN §8.4 |
| `conversation.update` for policy change | `[V1.x]` | CONVERSATION_PROTOCOL §7.1 |
| Multi-party UCTP beyond N=2 via SFU adapter | `[V2]` | PRD §5; INTERFACE_DESIGN §2.4 |
| SIP-over-QUIC / RoQ / MoQ adapters | `[V2]` | INTERFACE_DESIGN §2.5 |

> The earlier `rvoip-websocket` `[V1.x]` deferral is **closed** — the substrate
> shipped at `crates/uctp/rvoip-websocket`.

## Resolved design questions

The four v1 open questions were all decided as-shipped, and are recorded here so
they're not re-opened:

1. **`rvoip-harness` spin-off** — spun off as a separate crate (the seam was the point).
2. **`rvoip-identity` spin-off** — shipped as a separate crate with a no-op `BearerProvider`; the `IdentityProvider` trait stays in `rvoip-core::identity`.
3. **Tenant scoping** — P6 landed multi-tenant data isolation per process (registries + per-tenant quota).
4. **vCon production wiring** — P3 emits unsigned vCons by default; signing is gated behind `vcon-signing`. Production signing/encryption remains the `[V1.x]` item (not separately tracked above; folds into the vCon roadmap).
