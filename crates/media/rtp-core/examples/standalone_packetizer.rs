//! `RtpPacketSequencer` without `RtpSession` (standalone packetizer)
//!
//! Demonstrates building RTP packets for one SSRC — audio plus an RFC 4733
//! `telephone-event` (DTMF) tone sharing the same track — without an
//! `RtpSession`, a socket, a channel, or Tokio. Everything here is plain,
//! synchronous code: a caller that owns its own I/O (a `mio` reactor, raw
//! `sendmmsg`, or anything else) gets correct SSRC / sequence-number
//! bookkeeping without adopting the library's transport.
//!
//! Run with: `cargo run -p rvoip-rtp-core --example standalone_packetizer`

use bytes::Bytes;
use rvoip_rtp_core::{RtpPacket, RtpPacketSequencer};

const AUDIO_PAYLOAD_TYPE: u8 = 0; // PCMU
const DTMF_PAYLOAD_TYPE: u8 = 101; // RFC 4733 telephone-event
const CLOCK_RATE_HZ: u32 = 8_000;
const FRAME_DURATION_MS: u32 = 20;
const SAMPLES_PER_FRAME: u32 = CLOCK_RATE_HZ / 1_000 * FRAME_DURATION_MS;

/// Stand-in for whatever the caller actually uses to move bytes on the
/// wire (a `mio` UDP socket, `sendmmsg`, a Tokio socket, ...). The
/// packetizer never sees this — it only hands back an `RtpPacket`.
fn hand_off_to_transport(packet: &RtpPacket) {
    let bytes = packet.serialize().expect("serialize RTP packet");
    println!(
        "  -> pt={:<3} seq={:<5} ts={:<10} marker={:<5} {} bytes on the wire",
        packet.header.payload_type,
        packet.header.sequence_number,
        packet.header.timestamp,
        packet.header.marker,
        bytes.len(),
    );
}

fn main() {
    let ssrc = 0x1234_5678;
    let initial_sequence = 0;
    let mut sequencer = RtpPacketSequencer::new(ssrc, initial_sequence);

    println!("Audio (SSRC={ssrc:08x}), one packet per 20ms frame:");
    let mut audio_timestamp = 0u32;
    for _ in 0..3 {
        let payload = Bytes::from(vec![0u8; 160]);
        let packet = sequencer.packetize(AUDIO_PAYLOAD_TYPE, audio_timestamp, false, payload);
        hand_off_to_transport(&packet);
        audio_timestamp += SAMPLES_PER_FRAME;
    }

    // RFC 4733: every packet of one DTMF event keeps the event's *start*
    // timestamp — it does not advance like audio does — while the shared
    // sequence space (same `sequencer`) keeps incrementing normally. The
    // caller (not the sequencer) is responsible for RFC 4733 payload
    // encoding and for repeating the final packet per spec; this example
    // only shows the timestamp/sequence relationship.
    println!("\nDTMF event '5' (same SSRC, same sequence space, fixed timestamp):");
    let event_start_timestamp = audio_timestamp;
    let digit = 5u8;
    let end_of_event_flag = 0x80;
    for (duration, is_last) in [(160u16, false), (320, false), (320, true)] {
        let flags = if is_last { end_of_event_flag } else { 0 };
        let payload = Bytes::copy_from_slice(&[
            digit,
            flags,
            (duration >> 8) as u8,
            duration as u8,
        ]);
        let packet = sequencer.packetize(DTMF_PAYLOAD_TYPE, event_start_timestamp, is_last, payload);
        hand_off_to_transport(&packet);
    }

    // Audio resumes right after, same SSRC, sequence keeps counting from
    // wherever the DTMF event left it.
    println!("\nAudio resumes (sequence continues past the DTMF event):");
    let packet = sequencer.packetize(
        AUDIO_PAYLOAD_TYPE,
        event_start_timestamp + SAMPLES_PER_FRAME,
        false,
        Bytes::from(vec![0u8; 160]),
    );
    hand_off_to_transport(&packet);

    println!(
        "\nFinal sequencer state: ssrc={:08x} next_sequence={}",
        sequencer.ssrc(),
        sequencer.next_sequence()
    );
}
