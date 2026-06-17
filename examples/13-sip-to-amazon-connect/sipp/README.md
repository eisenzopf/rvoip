# SIPp end-to-end test (tone in, audio out, transcoder check)

Drives the example-13 gateway over real SIP to validate **both transcode
directions** of the SIP(G.711) ⟷ Amazon Connect(Opus) bridge:

- **Forward** — SIPp plays a 440 Hz G.711 tone into the gateway → transcoded to
  Opus → Amazon Connect → **you hear the tone on the agent CCP**.
- **Reverse** — the agent talks → Opus → gateway transcodes to G.711 → SIPp →
  we capture it and decode to **`return.wav`** you can play back.

## Files
- `make_tone.py` / `make-tone.sh` — generate `tone_pcmu.pcap` (PCMU RTP tone; no
  ffmpeg/sudo needed).
- `uac_tone.xml` — SIPp UAC scenario (offers PCMU, plays the tone, carries
  `X-Vapi-*` headers to exercise the attribute path, stays up 20 s, hangs up).
- `run-gateway.sh` — start the gateway (live AWS, loopback, agent flow).
- `run-sipp-test.sh` — capture return RTP + place the call + decode to WAV.

## Run

**Terminal 1 — gateway** (uses the same `connect-probe.env` you set up):
```bash
cd examples/13-sip-to-amazon-connect/sipp
./run-gateway.sh
# wait for: "SIP UAS on 127.0.0.1:5060"
```

**Terminal 2 — the call:**
```bash
cd examples/13-sip-to-amazon-connect/sipp
./run-sipp-test.sh        # enter your sudo password (tcpdump)
```
When the **agent CCP rings, answer it and talk** for ~10 s. You should hear the
440 Hz tone SIPp is playing. SIPp hangs up after 20 s.

**Result:**
```bash
afplay return.wav         # the agent's audio, transcoded Opus→G.711
```
- Hearing the tone on the CCP  ✅ G.711 → Opus works.
- `return.wav` contains your agent speech  ✅ Opus → G.711 works.

## Notes
- `return.wav` empty / "no PCMU RTP captured"? The agent didn't answer/talk, or
  media didn't bridge — check the gateway logs in terminal 1.
- The capture also prints an RTP-streams summary (packet counts each way) for a
  quick sanity check even without listening.
- Default target is `127.0.0.1:5060`; override with `DEST=host:port` / `MP=port`.
