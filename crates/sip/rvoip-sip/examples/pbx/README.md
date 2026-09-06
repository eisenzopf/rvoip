# Unified PBX Interop Examples

This directory is the source of truth for `rvoip-sip` PBX interop examples.
The same scenario code can run against Asterisk, FreeSWITCH, Jambonz, or the
registrar-proxy labs (Kamailio and OpenSIPS, each fronting an rtpengine media
relay) and through three public API surfaces:

- `Endpoint`: simple account/profile API
- `StreamPeer`: event-stream peer API
- `CallbackPeer::builder`: closure-based reactive callback API

## Setup

Copy the provider template you need and edit local addresses and credentials:

```sh
cp env/asterisk.env.example env/asterisk.env
cp env/freeswitch.env.example env/freeswitch.env
cp env/jambonz.env.example env/jambonz.env        # optional; up.sh writes the live values
cp env/kamailio.env.example env/kamailio.env      # optional; up.sh writes the live values
cp env/opensips.env.example env/opensips.env
```

`run.sh` also loads `examples/pbx/.env.local` when present. FreeSWITCH runs
also load `$HOME/Developer/freeswitch/freeswitch-local.env` when present, and
the Jambonz and proxy providers load
`$HOME/Developer/{jambonz,kamailio,opensips}/*-local.env`,
which their lab `up.sh` scripts write.

## Runner

```sh
./run.sh --pbx asterisk --api all --scenario registration
./run.sh --pbx freeswitch --api all --scenario hold_resume
./run.sh --pbx jambonz --api all --scenario all --transport UDP
./run.sh --pbx both --api all --scenario all
```

Options:

- `--pbx asterisk|freeswitch|jambonz|both|kamailio|opensips|proxies|all`
  (`both` deliberately stays Asterisk+FreeSWITCH so existing release matrices
  are unchanged; `proxies` is the two registrar-proxy labs; `all` is every
  provider)
- `--api endpoint|stream_peer|callback|all`
- `--scenario registration|basic_call|g729_call|amr_call|amr_transcode_call|b2bua_call|hold_resume|ring_cancel|dtmf|reject|blind_transfer|all`

The runner builds the PBX Cargo examples and stores logs/WAV evidence under
`examples/pbx/output/<provider>/<api>/<scenario>/<transport>/` by default. Set
`PBX_OUT_ROOT=/path/to/artifacts` to write the same evidence tree somewhere
else, which is what the beta gate does.

`g729_call` builds the examples with `dev-insecure-tls,g729` by default and
runs both `PBX_CODEC_PROFILE=g729a` and `PBX_CODEC_PROFILE=g729ab` unless a
single `PBX_CODEC_PROFILE` is provided. `g729a` advertises PT 18 with
`a=fmtp:18 annexb=no`; `g729ab` advertises `a=fmtp:18 annexb=yes`. Set
`PBX_G729_PROFILES="g729a g729ab"` to customize the profile list used by the
matrix and beta gate.

`g729_call` is an audio-verifying scenario. Both endpoints send distinct
reference tones through the negotiated G.729 media path, record the received
audio, and run the analyzer before the matrix cell passes. G.729 evidence is
stored under `.../g729_call/<profile>/<transport>/` with `audio-analysis.*`
diagnostics when `PBX_DIAG=1`.

`amr_call` and `amr_transcode_call` are part of `--scenario all` and are
audio-verifying under a stricter gate (tone dominance plus per-window SNR and
per-frame level, one continuous second, at the leg's own rate). Whether they
*run* depends on the PBX **image**, not the provider: the local labs carry
AMR, the committed release-runner images do not. Before each AMR cell,
`amr_probe.sh` asks the container (`core show codecs` / `show codec`) and
records a `SKIP` matrix row when the needed variant is absent, with the probe
transcript as the row's log. Three knobs, all recorded in `environment-*.md`:

- `PBX_ASSUME_AMR=0|1` pins the answer without touching docker — the release
  gates pin `0` so gate behaviour is deterministic.
- `PBX_REQUIRE_AMR=1` (set by the AMR-capable labs' env files) turns any skip
  into a loud FAIL, so a lab losing its codec cannot hide as a skip.
- Both knobs given on the command line override values from the sourced env
  files; for everything else the files win.

`b2bua_call` puts **rvoip in the middle**: caller(2001) → PBX → rvoip b2bua(2002)
→ PBX → target(2003), three role processes, with rvoip terminating both legs and
bridging their payloads through `UnifiedCoordinator::bridge`. Two independent PBX
calls exist, so nothing joins the two ends except rvoip; if the PBX transcoded or
re-framed a leg, the bridge would refuse (`CodecMismatch`/`FormatMismatch`) and
the cell would fail. It sweeps a PCMU control cell plus AMR-WB in the framing the
PBX can relay (Asterisk octet-aligned, FreeSWITCH bandwidth-efficient),
overridable with `PBX_B2BUA_PROFILES`. Endpoint API only — the other two APIs
record no rows for it. Evidence under `.../b2bua_call/<profile>/<transport>/`; the
b2bua middle node records nothing, so 880 Hz appearing anywhere would itself be a
fault.

Each run also writes release-audit artifacts at the output root:

- `environment-*.md`: host, toolchain, git revision, SIPp/tshark availability,
  selected runner arguments, and redacted runtime environment.
- `matrix.tsv`: one row per provider/API/scenario/transport/role command with
  pass/fail status, duration, exit code, log path, and output directory.
- `summary.md`: markdown summary of the matrix suitable for attaching to beta
  release evidence.
- `<provider>/<api>/<scenario>/<transport>/*_metadata.md`: per-cell command and
  redacted environment details next to the raw logs and media artifacts.

## Cargo Examples

The runner orchestrates these examples by setting `PBX_PROVIDER`,
`PBX_SCENARIO`, `PBX_TRANSPORT`, and `PBX_ROLE`.

```sh
cargo run -p rvoip-sip --features dev-insecure-tls,g729 --example pbx_stream_peer
cargo run -p rvoip-sip --features dev-insecure-tls,g729 --example pbx_endpoint
cargo run -p rvoip-sip --features dev-insecure-tls,g729 --example pbx_callback_builder
cargo run -p rvoip-sip --features dev-insecure-tls,g729 --example pbx_analyze
```

## Scenario Matrix

The unified suite exercises these scenarios against both PBXs and all three API
surfaces:

- registration/unregistration for TLS `1001` and UDP `2001`
- basic UDP call `2001 -> 2002` for G.711 PCMU/PCMA interop with
  bidirectional tone audio verification
- G.729A and G.729AB UDP call `2001 -> 2002` with PT 18, Annex B SDP coverage,
  and bidirectional audio verification
- UDP and TLS/SRTP hold/resume
- UDP and TLS/SRTP ring/cancel
- UDP and TLS/SRTP DTMF
- UDP and TLS/SRTP reject/busy
- UDP and TLS/SRTP blind transfer to `2003`/`1003`

Asterisk registered-flow TLS/SRTP is provider-gated: set
`ASTERISK_TLS_CONTACT_MODE=registered-flow-symmetric` or
`ASTERISK_TLS_FLOW_REUSE=1` and run the TLS scenarios.

## Jambonz B2BUA Lab

`infra/release-runners/pbx/jambonz` runs Jambonz as a first-class release
peer, at the same policy level as Asterisk and FreeSWITCH. The 0.3.10 profile
selects the latest stable open-source component line (currently 0.9.9), checks
that both SBC pins are still upstream HEAD, verifies source tarball hashes,
and runs only digest-pinned containers. The test target is the actual Jambonz
outbound SBC, registrar, Drachtio server, database, Redis, and RTPengine—not a
mock that merely copies Jambonz responses.

The mandatory profile uses the same `rvoip-sip/examples/pbx` runner and
scenario implementations as Asterisk and FreeSWITCH. It runs all three RVoIP
public SIP APIs over UDP and covers authenticated registration, separate PCMU
and PCMA calls, DTMF, cancellation, rejection, hold/resume, transfer, and
teardown, with RVoIP in both caller and callee roles. The pinned Jambonz OSS
profile explicitly admits only PCMU, PCMA, and telephone-event, so its gate
does not claim G.729, AMR, TLS/SRTP, or the separate RVoIP-as-B2BUA scenario;
those codecs remain covered by the Asterisk and FreeSWITCH release profiles.
Every run records the selected component version/revisions, image identities,
rendered topology, redacted logs, matrix, media assertions, and a cleanup
receipt. Diagnostic runs can additionally capture packets. A passing matrix
is a claim about SIP/SDP/RTP
interoperability with that exact open-source Jambonz profile; it is not a
claim about the commercial Jambonz verb API or hosted jambonz.cloud.

Bring the release lab up or down with:

```sh
bash infra/release-runners/interop-lifecycle.sh jambonz-up
bash infra/release-runners/interop-lifecycle.sh jambonz-down
```

The pinned Jambonz and MySQL images require an amd64 Docker engine. The
mandatory release gate uses the repository's x86 GCP interop worker. On Apple
Silicon, run this lab with a dedicated x86_64 Colima profile using the gRPC
port forwarder (Colima's default SSH forwarder does not carry UDP):

```sh
brew install qemu lima-additional-guestagents
colima start --profile rvoip-release-x86 --arch x86_64 \
  --cpu 6 --memory 12 --disk 30 --port-forwarder grpc
docker context use colima-rvoip-release-x86
```

`up.sh` fails early on the wrong architecture or when a live UDP echo probe
cannot traverse the selected Colima forwarder. The Colima profile publishes
Drachtio's SIP listener and a bounded RTPengine range only on macOS loopback,
while advertising `host.docker.internal` for the return signaling and media
path into RVoIP. It does not assume that macOS can route directly to Docker's
private bridge. Linux release workers use the isolated bridge directly.

Jambonz TLS/SRTP, WebRTC, CPaaS application verbs, hosted service, PSTN, HA,
recording, and load are outside the 0.3.10 Jambonz profile and must not be
inferred from this gate.

## Registrar-Proxy Labs (Kamailio, OpenSIPS + rtpengine)

`infra/release-runners/pbx/{kamailio,opensips}` bring up a registrar-proxy
with an rtpengine media relay (`up.sh` / `down.sh`, or
`infra/release-runners/interop-lifecycle.sh {kamailio,opensips}-{up,down}`).
These are a different oracle class from the B2BUAs: the proxy stays in the
signaling path via Record-Route while rtpengine relays RTP **verbatim** — the
`rtpengine_manage` flags deliberately carry no transcode/codec options, so an
AMR payload crosses untouched in any framing. That is what the AMR exit
criterion's "Kamailio+rtpengine" peer proves: our AMR flows through a relay
that never re-encodes it, in all four framings
(`amrnb amrwb amrnb_be amrwb_be` — `amr_profile_list` sweeps all four for
proxies since one config relays them all).

Proxy provider notes:

- v1 scenario set is `registration`, `basic_call`, and `amr_call`; the rest
  sit behind `PBX_PROXY_ALL_SCENARIOS=1` and `amr_transcode_call` never runs
  (rtpengine transcoding is out of scope).
- **TLS+SRTP runs against Kamailio** (port 5073, `tls.so` plus a self-signed
  certificate `up.sh` regenerates per run; `enable_tls=1` is required or
  Kamailio only *warns* and answers nothing). The endpoints negotiate SDES
  end-to-end and rtpengine relays the encrypted payload without keys of its
  own — it logs one "SRTP output wanted, but no crypto suite was negotiated"
  per session and then forwards transparently, which is the passthrough
  property working rather than failing. The proof is that the tone gate passes
  at both ends: that needs each endpoint to decrypt the other's SRTP, which
  cannot happen unless the crypto attributes crossed verbatim.
- **rtpengine transcoding is opt-in** and is a different claim from the
  passthrough labs. Bring the lab up with `PBX_PROXY_TRANSCODE=1` and
  `amr_transcode_call` becomes available: Kamailio then passes
  `codec-transcode-*` flags, rtpengine decodes our AMR with its own
  opencore-amr and re-encodes to PCMU, and the far leg is tone-verified. That
  makes rtpengine a **third independent foreign decoder** of our AMR, after
  Asterisk and FreeSWITCH. The cell is gated twice — on the flag and on
  probing `rtpengine --codecs` — because without the flags it would pass while
  proving nothing, and without the codecs it would fail for a reason that is
  not ours.
  - AMR-NB ↔ PCMU works. **AMR-WB ↔ PCMU does not, and the cause is on
    rtpengine's side of the boundary, not ours.** With
    `--log-level-codec=7` rtpengine's own decisions show it building
    `PCMU/8000 -> AMR-WB/16000` for one direction and then choosing a
    *passthrough* handler for the inbound AMR-WB ("Sink supports codec
    AMR-WB/16000"), so an `AMR-WB -> PCMU` decoder is never created and the
    PCMU leg receives nothing. It then logs "Eliminating asymmetric inbound
    codec AMR-WB/16000". Every one of those is a negotiation decision taken
    before a single byte of our media is examined.
    Two independent facts confirm our wideband stream is not the problem:
    Asterisk transcodes the identical scenario with its own opencore-amrwb
    (tone-verified, 16160 samples on the PCMU leg), and rtpengine itself
    relays the same stream untouched in the passthrough lab, where both
    endpoints decode it. What is still open is whether this is an rtpengine
    limitation with mixed 8/16 kHz clock rates or a flag incantation not yet
    found — `codec-transcode-AMR-WB` in any spelling makes it worse, because
    it tells rtpengine the PCMU side speaks AMR-WB.
    The wideband pairing is therefore left out of the default list rather
    than shipped red; `PBX_AMR_TRANSCODE_PAIRINGS` forces it for anyone
    picking the thread up.
- **OpenSIPS stays UDP-only.** Its stock 3.6 image ships no TLS modules; the
  pinned-deb TLS image the sip-proxy suite uses is what would unblock it
  (see `crates/sip/sip-proxy/docs/OPENSIPS_TLS_PROVENANCE.md`), and
  `provider_supports_tls` returns false for it so TLS cells skip rather than
  fail.
- The proxy configs **fail closed**: if `rtpengine_manage` cannot reach the
  relay the INVITE is answered 503 rather than relayed with untouched SDP —
  otherwise media would flow endpoint-to-endpoint and every media assertion
  would pass vacuously. The lab `up.sh` readiness gates on the registrar
  answering AND the rtpengine node being enabled.
- Because the proxy stays in the path, these labs also exercise Route-set
  discipline: the 200's Record-Route teaches our UAC its route set and the
  BYE traverses the proxy (watch `Received command 'delete'` in the
  rtpengine log — one per call teardown).
- `PBX_DIAG=1` snapshots `kamcmd ul.dump` / `rtpengine.show all`
  (`opensips-cli -x mi ul_dump` / `rtpengine_show`) before and after each
  cell, tails the rtpengine log, and the cell pcap shows the AMR PTs and
  `a=fmtp` crossing the relay unchanged with only address/port rewritten.
- Ports: Kamailio SIP 5072 / ng 2223 / RTP 23000-23200 / endpoint bases
  35070+; OpenSIPS 5074 / 2224 / 23300-23500 / 45070+. All four labs and the
  sip-proxy interop suite (25070+) can coexist.
- The beta gate runs these behind `BETA_RUN_PROXY_PBX=1` (default skip).

## Endpoint Notes

The shared harness constructs one `SipAccount` per participant and derives
`Config.credentials`, `Registration`, `EndpointAccount`, and the StreamPeer /
CallbackPeer registration builders from that account. This keeps PBX digest
username, AOR username, password, From URI, Contact URI, and expiry in one
place across the three API surfaces.

`Endpoint` intentionally remains a simple account API. Advanced scenarios still
use `SessionHandle` for per-call operations such as `hold`, `resume`,
`send_dtmf`, and `transfer_blind_and_wait_for_outcome`. Ring/cancel and reject
use `Endpoint::wait_for_incoming` plus `IncomingCall` decisions. This keeps the
simple Endpoint setup path under test while documenting where advanced
operations belong on the per-call handle.

## Provider Differences

Provider-specific differences are encoded in config defaults and capability
flags, not duplicated scenario code:

- Asterisk defaults to `SIP_PORT=5060`, `SIP_TLS_PORT=5061`,
  `SIP_PASSWORD=password123`, longer registration settle/retry windows, and
  optional registered-flow operation. Asterisk TLS blind-transfer tests use a
  longer default REFER settle window; override with
  `ASTERISK_TLS_TRANSFER_SETTLE_SECS` when needed.
- FreeSWITCH defaults to `FREESWITCH_UDP_ADDR=127.0.0.1:5062`,
  `FREESWITCH_TLS_ADDR=127.0.0.1:5063`, `FREESWITCH_PASSWORD=1234`,
  `15070/15080` local SIP ports, and `SrtpSuitePolicy::FreeSwitchCompatible`.
- Asterisk target-side CANCEL may not always be surfaced by the PBX profile, so
  caller-side cancel remains the required assertion unless
  `ASTERISK_EXPECT_TARGET_CANCEL=1` is set.
