# TURN for rvoip — delivery plan

Status: **plan, for owner review** · 2026-08-19 · branch `thelve/rvoip-22-ingress`
Companion: `docs/ICE_IMPLEMENTATION_PLAN.md` (implemented §12; this document
expands its deferred phase I7). Standards: RFC 8656 (TURN, obsoletes 5766),
RFC 8489 (STUN long-term credentials), RFC 8839 (relay candidates in SDP).

---

## 1. What TURN is, and where it sits in SIP

TURN is not a SIP mechanism. It is a **STUN extension that plugs into ICE as
one more candidate type**: the client asks a relay server to allocate it a
public transport address, and media to and from the peer flows through that
relay. In SIP terms the only visible change is one more line in the SDP —
`a=candidate ... typ relay` — and one more column in the ICE check matrix.
Signaling is untouched.

Relay candidates carry **type preference 0**, the lowest (RFC 8445
§5.1.2.2). The agent tries host and reflexive paths first and lands on the
relay only when nothing better validates. TURN is pure fallback — it is
never the preferred path, and a call that can go direct always will.

## 2. Verified inventory (2026-08-19, at `28d772db`)

- **Browser legs already have TURN.** `rvoip-webrtc`'s `IceServerConfig::turn`
  configures relay servers on the webrtc-rs stack. This plan is therefore
  purely about the **SIP/RTP path**: Thelve's BYO callers on hostile
  networks, and rvoip's library roles (UAC, UA↔UA).
- **The candidate model is ready.** `ice-core`'s `CandidateKind::Relayed`
  exists with type preference 0 and the SDP `relay` token; the agent's pair
  logic needs no remodeling to admit relayed candidates.
- **The STUN codec is short-term-only.** No REALM, NONCE, or long-term
  MESSAGE-INTEGRITY keying — and TURN authenticates exclusively with the
  long-term mechanism. This is the gate to everything else.
- **The transport seam exists.** `RtpEvent::StunPacket` +
  `RtpTransport::send_stun_bytes` (from the ICE work) are the demux/inject
  points a relay data path will extend.

## 3. The value, stated per topology — because it is very uneven

### Where it adds nothing

**A NAT'd caller reaching Thelve's public gateway.** This is Thelve's main
case today, and TURN does not help it because it does not need help: when
one side is public, even the nastiest address/port-dependent ("symmetric")
NAT on the caller works. The caller's own checks toward the public gateway
create the NAT mapping, and the gateway replies from the exact address the
caller hit. Our ice-lite mode already closes this. **This is why TURN was
deferred** — for the server-anchored topology, srflx is sufficient.

### Where it is the only thing that works

1. **UDP-hostile networks calling anything — including Thelve.** A corporate
   firewall that blocks outbound UDP entirely (or permits only TCP/TLS to
   :443) defeats every direct and reflexive path; the gateway being public
   is irrelevant if no UDP leaves the building. TURN-over-TCP/TLS on 443 is
   the industry answer. **Note: this value lives in the TCP/TLS transport
   phase (P4), not the UDP relay phase.** The pitch in one sentence: *the
   caller in the locked-down office still gets audio.*
2. **UA↔UA with symmetric NAT on either side** — the library roles the owner
   directed rvoip to serve. Two full agents behind address/port-dependent
   NATs can never validate a reflexive pair: each side's mapped port toward
   the STUN server differs from its mapped port toward the peer, so checks
   never land. Relay is the only path. Industry-wide, roughly 10–15% of
   peer-to-peer sessions end up on relay, skewed hard toward mobile CGNAT
   and corporate networks.
3. **Privacy, as a side effect**: relayed media hides each endpoint's real
   address from the other.

## 4. How we deliver it

Two halves. We **build the client**; we **deploy the server**.

### The client — in `ice-core`, same sans-io shape as the agent

**P1 — long-term credentials in the STUN codec [M].** The gate to all of
it. TURN authenticates with RFC 8489's *long-term* mechanism: an initial
request is met with 401 + REALM + NONCE; the retry keys MESSAGE-INTEGRITY
with `MD5(username ":" realm ":" password)`; a 438 stale-nonce answer
triggers a re-key with the fresh nonce. New methods for the codec —
Allocate (0x003), Refresh (0x004), Send/Data indications (0x006/0x007),
CreatePermission (0x008), ChannelBind (0x009) — and new attributes:
REQUESTED-TRANSPORT, XOR-RELAYED-ADDRESS, XOR-PEER-ADDRESS, LIFETIME, DATA,
CHANNEL-NUMBER, REALM, NONCE, ERROR-CODE additions (401/438/486/508).
Acceptance: the 401→retry and 438→re-key dances as scripted tests; codec
vectors round-trip.

**P2 — the allocation state machine [M–L].** Sans-io, like the agent:
allocate → learn the relayed address → hand the agent a
`Candidate::Relayed` (base = the relayed address, related = the local
socket); refresh at lifetime/2; **CreatePermission for each remote peer
before any check or media can flow to it** (the relay drops unpermitted
traffic — a correctness rule, not an optimization); upgrade the selected
pair to ChannelBind, which drops per-packet overhead from 36 bytes
(Send indication) to 4 (channel data). Failure honesty: allocation lost
(refresh 437) → the relayed candidate dies; if it carried the selected
pair, the agent re-nominates from surviving valid pairs or fails — never
silently blackholes. Scripted tests: expiry, refresh races, permission
lifetimes (5 min), channel rebind, allocation-lost mid-call.

**P3 — the media encapsulation path in rtp-core [M — the riskiest piece].**
Everywhere else, ICE only *retargets an address*; a relayed pair changes
the **shape** of the media path. Outbound RTP must be wrapped (channel data
framing, or Send indications before a channel exists) and sent to the
*relay server's* address; inbound arrives wrapped and must be unwrapped
before the RTP session sees it. Design: a relay adapter at the transport
layer — the same seam the SRTP wrapper uses — activated when the selected
local candidate is relayed. SRTP composes cleanly: the SRTP-protected
packet goes *inside* the TURN framing (encrypt-then-relay), so the relay
never sees plaintext media.

**P4 — TCP and TLS transport to the relay [M].** RFC 8656 allows the
client↔relay leg over TCP/TLS while the relay↔peer leg stays UDP. This is
the firewall-traversal value (§3.1) and is deliberately a separate phase:
UDP-relay first proves the state machine; the stream framing (RFC 4571
length-prefixing) and TLS session management land on top of it.

**P5 — agent + Thelve exposure [S–M].** Gathering integration is small (the
model is ready). Facade: `SipConfig::turn(TurnServerConfig { url,
credentials })`. Thelve: chart `realtimeIngress.media.turn` — URLs plus
**secret-ref credentials only**, using the ephemeral-credential scheme
(time-limited HMAC usernames from a shared secret — coturn's
`use-auth-secret` pattern) so no static relay password ever ships in
config, per Thelve's standing credential discipline.

### The server — deployed, not built

Run **coturn**, the standard, battle-tested relay: a container recipe
(ports 3478/udp+tcp, 5349/tls, the relay port range, `use-auth-secret`
shared with Thelve's credential minting) joins the deployment docs.

Whether rvoip should eventually *be* a TURN server — plausible for the SBC
role, where a public rvoip edge relays for its own clients — is explicitly
**out of scope** until the client exists and a deployment wants it.

## 5. Scope summary and order

| Phase | Delivers | Size |
|---|---|---|
| P1 codec long-term credentials | unlocks everything | M |
| P2 allocation state machine | relayed candidates exist | M–L |
| P3 media encapsulation | **UA↔UA across symmetric NAT works** | M (riskiest) |
| P4 TCP/TLS to relay | **the locked-down-office caller works** | M |
| P5 facade + Thelve + coturn recipe | deployable | S–M |

Total ≈ the ICE agent build itself (L–XL), phasing cleanly with value at
P3 and again at P4. Recommendation unchanged from the ICE plan: build when
a concrete deployment needs it — but if built proactively, this order
front-loads the capability that has **no substitute** (symmetric↔symmetric)
and defers nothing that lite/full ICE don't already cover.

## 6. Non-goals

- **RFC 6062 (TCP allocations)** — relaying TCP *media*; nothing in scope
  needs it.
- **rvoip-as-TURN-server** — see §4; revisit with the SBC role.
- **Browser legs** — already served by webrtc-rs's TURN support.

## 7. Open questions for the owner

1. Build proactively now, or on first concrete need (a customer behind
   full-UDP-block, or a UA↔UA product requirement)?
2. If proactively: is P4 (TCP/TLS) required for the first cut, or is
   UDP-relay-first acceptable? P4 is where the Thelve-side value is.
3. coturn as the blessed relay, or is there appetite for an rvoip-native
   relay on the SBC roadmap (changes nothing here, but changes what P5
   documents)?
