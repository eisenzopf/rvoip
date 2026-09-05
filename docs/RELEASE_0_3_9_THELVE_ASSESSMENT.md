# 0.3.9 plan — assessment from the Thelve side

Status: assessment, 2026-08-18 · Reviews `RELEASE_0_3_9_PLAN.md` (@ `ed8100ae`)
against Thelve's telephony direction: a managed / BYO-SIP provisioning menu
with rvoip as the SIP edge (`Thelve:docs/architecture/telephony-managed-and-byo-sip-plan.md`).
Spot-checks of the plan's claims were verified independently against the
`v0.3.8` tag and the Thelve tree.

## Verdict

**0.3.9 as scoped is the right release for the BYO-SIP product.** Its
workstreams are, almost item for item, the difference between "a trunk you
can demo" and "a trunk a carrier will accept" — and Thelve's own gating
already assumes exactly this split (dedicated-tenant pilots now, carrier
grade at the 0.3.9 re-pin, Cloud Shared at 0.4.0). Three adjustments are
recommended, in priority order:

1. **Run the #93 severity triage immediately — Thelve is the deployment its
   conditional describes.** (§3)
2. **Extract the RVOIP-22 composition change from the 0.4.0 runtime cluster
   and size it on its own.** It is the single item gating Thelve's shared-
   cloud BYO SIP, and it is plumbing, not the per-call-ownership spine it is
   currently bundled with. (§4)
3. **Hold the plan's own big-rock sizing discipline early** (#168 + #202 +
   #203 together), because Thelve's "pilot → carrier grade" relabel depends
   on Workstreams 1–2 landing as a set; a partial landing would leave the
   relabel dishonest. The plan's cut order (#202 → #203) is right from the
   Thelve side too: NAT traversal without DTLS still serves SIP trunks;
   DTLS without ICE serves almost nothing Thelve needs first.

## 1. What each workstream means for the telephony product

| 0.3.9 item | Thelve meaning on a BYO trunk |
|---|---|
| #168 jitter buffer on the SIP receive path | Today the SIP path plays audio with **no jitter buffer and no PLC** (the plan's own audit). This is the single biggest gap between pilot and production; without it every other quality claim is moot. Rightly the release's big rock. |
| #200 RTCP XR + honest MOS | Per-call loss/jitter/RTT/MOS flow into the stats API Thelve already projects into call evidence — carriers and tenants get quality records instead of a hardcoded 4.5. |
| #201 P-headers in the trust domain | Inbound caller identity (PAI) accepted only from `trusted_trunk` peers, provenance-marked. Thelve's per-tenant trunk-trust rows (CIDRs + TLS subject) map 1:1 onto this; caller identity on BYO trunks starts existing. |
| #202 DTLS-SRTP, #203 ICE | Encrypted media and NAT traversal — the difference between "works from an SBC on a static IP" and "works from customers as they actually are." Shared demux layer makes them cheaper together, as the plan says. |
| #204 tolerant RTCP walker, #184/#185 | Interop correctness with real peers; #204 specifically keeps #200's pipeline fed on exactly the trunks that matter. |
| #198 partial shard results | Protects the qualification cycles every Thelve re-pin ceremony depends on. Cheap, high leverage — agree with "immediately." |

Independent verification note: the plan's central honesty claims held up.
`v0.3.8` was inspected directly — it contains the AMR/SIPS/interop content
and none of the app-runtime surface; the `RvoipAppBuilder` at the tag has no
pre-adapter admission-gate or operational-stream composition.

## 2. What 0.3.9 deliberately does not unlock — keep it explicit

Multi-tenant lossless ingress (the RVOIP-22 ask: admission gate + operational
stream installable before adapter registration, fail-closed on receiver
loss) stays in the 0.4.0 "RvoipApp production runtime" cluster. Consequence,
already encoded in Thelve's plan: BYO SIP ships for `cloud_dedicated` /
`private_*` tenants on the fixed-tenant profile, and `cloud_shared` waits.
That is a fine schedule **as long as it is a decision, not an accident** —
hence §4.

## 3. #93 (AAuth) — triage inputs from the Thelve side

The plan conditions severity on: *"if any deployment gates AI tool payloads
on AAuth today, the scope union is privilege escalation and ships as a point
fix."* Facts from the Thelve tree:

- `thelve-aauth` consumes `rvoip_auth_core::{KeyResolver, Sig9421Verifier}`,
  `sig9421::jcs_canonicalize`, and matches `Sig9421Error::ReplayDetected` —
  Thelve's entire signed AI door rides this module. Two of #93's three
  verified findings live in it (no upper clock bound; process-local
  get-then-insert replay cache).
- Mitigations already on the Thelve side, which the triage should credit:
  delegation spend is PostgreSQL consume-once
  (`consume_aauth_delegated_capability`) and capability invocations carry
  durable idempotency — so replay at the *capability* layer has a
  database-backed backstop even where the signature-layer cache is weak.
- The **missing upper clock bound** (far-future `created` passes) has no
  Thelve-side compensation at the signature layer unless Thelve's envelope
  validation independently bounds it — verify this one way or the other.
- The **scope union** is likely inapplicable: Thelve computes effective
  authority from its own delegation/role model, not from rvoip scope
  claims — verify, then record it, so the finding is scoped to deployments
  that do use rvoip's scope evaluation.

Recommendation: the upper-clock-bound fix ships as the point fix regardless
of the union verdict; the replay-cache hardening can ride 0.3.9 with its
severity honestly downgraded for database-backed consumers like Thelve.

## 4. The RVOIP-22 extraction argument

The release theme is *"the primitive exists, and nothing reaches it."*
RVOIP-22 is the same disease in the app layer: the admission gate and the
backpressured operational stream **exist in core** and the builder's
construction order makes them unreachable (`InvalidState` after `build`).
It is composition plumbing with an acceptance matrix already written in
Thelve's handoff register (capacity-one streams, CANCEL/BYE races, receiver
loss → degraded readiness, bounded drain). It is not the per-call-ownership
spine, the per-call API, or the rest of the 0.4.0 cluster it is currently
filed under.

Recommendation: split it into its own sized issue now. If the big-rock
sizing pass leaves room, it is the highest-leverage stretch item 0.3.9
could carry; if not, it should be the *first* item of the next release
rather than waiting on the full runtime cluster. Every month it waits is a
month Thelve's shared-cloud BYO SIP — the actual CCaaS product surface —
stays closed for a reason the theme of this very release argues against.

## 5. What Thelve builds regardless (no rvoip dependency)

Thelve's plan phases B–E — trunk-trust model, BYO number registration,
shared DID-routing hook, gateway dispatch arm, provisioning menu, Helm RTP
exposure — are pin-independent and proceed on 0.3.8. The 0.3.8 re-pin
(codec/SIPS content; closes no register items — verified) is mid-flight in
the Thelve tree. The 0.3.9 and 0.4.0 re-pins are scheduled ceremonies that
flip quality and deployment labels, not rebuilds.
