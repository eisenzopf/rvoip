# RTP/RTCP correctness migration for 0.3.5

The 0.3.5 correctness repair preserves RTP padding as explicit packet
metadata. This intentionally changes two public data shapes:

- `RtpPacket` has a public `padding_size: u8` field. Downstream struct
  literals must set it to `0` for an unpadded packet, or use `RtpPacket::new`
  and `set_padding`.
- `RtpEvent::MediaReceived` has a `padding_size: u8` field. Exact match
  patterns must bind that field or include `..`.

Parsed packet payloads continue to exclude padding bytes. A non-zero padding
size means the RTP P bit is set, and serialization writes canonical padding
whose final octet contains the count.

`RtpStreamStats::jitter` is now reported in RTP timestamp units, as required
for RTCP report blocks. Use `RtpSessionStats::jitter_ms` when milliseconds are
needed. `RtpStreamStats::packets_out_of_order` exposes the reordered-packet
counter that was previously internal.
