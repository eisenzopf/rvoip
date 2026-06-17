#!/usr/bin/env python3
"""Synthesize a 440 Hz tone, encode to G.711 mu-law, and emit a text2pcap input
file describing it as a PCMU (PT 0) RTP stream.

Usage: make_tone.py <seconds> <out.txt>

Self-contained (math + a standard mu-law encoder) so it needs no ffmpeg/sox.
text2pcap turns the output into tone_pcmu.pcap; SIPp's play_pcap_audio replays
it. Timestamps are spaced 20 ms apart for real-time pacing.
"""
import math
import sys

RATE = 8000
FRAME = 160  # 20 ms
FREQ = 440.0
SSRC = 0x11223344


def linear2ulaw(sample: int) -> int:
    """ITU-T G.711 mu-law encode of a 16-bit linear PCM sample."""
    BIAS = 0x84
    CLIP = 32635
    sign = 0x80 if sample < 0 else 0x00
    if sign:
        sample = -sample
    if sample > CLIP:
        sample = CLIP
    sample += BIAS
    exponent = 7
    mask = 0x4000
    while (sample & mask) == 0 and exponent > 0:
        exponent -= 1
        mask >>= 1
    mantissa = (sample >> (exponent + 3)) & 0x0F
    return (~(sign | (exponent << 4) | mantissa)) & 0xFF


def main() -> None:
    seconds = float(sys.argv[1])
    out = sys.argv[2]
    total = int(RATE * seconds)

    # Synthesize mu-law audio (half-scale sine to avoid clipping).
    ulaw = bytearray(total)
    for n in range(total):
        s = int(16000 * math.sin(2 * math.pi * FREQ * n / RATE))
        ulaw[n] = linear2ulaw(s)

    lines = []
    seq = 0
    ts = 0
    nframes = total // FRAME
    for i in range(nframes):
        payload = ulaw[i * FRAME : (i + 1) * FRAME]
        hdr = bytes(
            [
                0x80,
                0x00,  # PT 0 (PCMU)
                (seq >> 8) & 0xFF,
                seq & 0xFF,
                (ts >> 24) & 0xFF,
                (ts >> 16) & 0xFF,
                (ts >> 8) & 0xFF,
                ts & 0xFF,
                (SSRC >> 24) & 0xFF,
                (SSRC >> 16) & 0xFF,
                (SSRC >> 8) & 0xFF,
                SSRC & 0xFF,
            ]
        )
        pkt = hdr + bytes(payload)

        secs = i * 0.02
        h = int(secs // 3600)
        m = int((secs % 3600) // 60)
        s = secs % 60
        lines.append(f"{h:02d}:{m:02d}:{s:09.6f}")  # text2pcap -t "%H:%M:%S."
        for off in range(0, len(pkt), 16):
            chunk = pkt[off : off + 16]
            lines.append(f"{off:06x}  " + " ".join(f"{b:02x}" for b in chunk))
        lines.append("")

        seq = (seq + 1) & 0xFFFF
        ts = (ts + FRAME) & 0xFFFFFFFF

    open(out, "w").write("\n".join(lines) + "\n")
    sys.stderr.write(f"wrote {nframes} RTP packets ({seconds}s) to {out}\n")


if __name__ == "__main__":
    main()
