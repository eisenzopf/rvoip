//! Media Synchronization Example
//!
//! This example demonstrates how to synchronize multiple RTP streams
//! (audio and video) with different clock rates.

use bytes::Bytes;
use std::time::Duration;
use tokio::time;
use tracing::{info, debug, error, warn};
use rvoip_rtp_core::{
    RtpSession, RtpSessionConfig, RtpSessionEvent,
    MediaSync, TimestampMapper, MediaClock,
    packet::rtcp::{RtcpPacket, RtcpSenderReport, NtpTimestamp},
};

const AUDIO_SSRC: u32 = 0x1234ABCD;
const VIDEO_SSRC: u32 = 0x5678DCBA;
const AUDIO_CLOCK_RATE: u32 = 48000; // 48kHz audio
const VIDEO_CLOCK_RATE: u32 = 90000; // 90kHz video

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();
    
    info!("Starting media synchronization example");
    
    // Create a timestamp mapper for synchronizing streams
    let mapper = TimestampMapper::new();
    
    // Register audio and video streams with their clock rates
    mapper.register_stream(AUDIO_SSRC, AUDIO_CLOCK_RATE, 0);
    mapper.register_stream(VIDEO_SSRC, VIDEO_CLOCK_RATE, 0);
    
    // Create simulated RTCP SR information
    let initial_ntp = NtpTimestamp::now();
    let audio_rtp_ts = 0;
    let video_rtp_ts = 0;
    
    // Update mapper with initial timing information
    mapper.update_from_sr(AUDIO_SSRC, initial_ntp, audio_rtp_ts);
    mapper.update_from_sr(VIDEO_SSRC, initial_ntp, video_rtp_ts);
    
    // Map streams to each other
    info!("Creating initial mapping between audio and video streams");
    mapper.map_streams(
        AUDIO_SSRC, VIDEO_SSRC, 
        audio_rtp_ts, video_rtp_ts, 
        initial_ntp
    );
    
    // Simulate media capture with lip sync issues
    info!("Simulating media capture with 100ms lip sync issues");
    
    // Audio is captured at perfect timing
    let audio_capture_timestamp = AUDIO_CLOCK_RATE * 1; // 1 second in
    
    // Video is slightly delayed (100ms) - this is our lip sync issue to fix
    let video_capture_timestamp = VIDEO_CLOCK_RATE * 1 - 9000; // 900ms in (100ms behind audio)
    
    // Calculate current wallclock time for both streams based on their RTP timestamps
    let audio_wallclock = mapper.rtp_to_wallclock(AUDIO_SSRC, audio_capture_timestamp)
        .expect("Failed to convert audio timestamp to wallclock time");
        
    let video_wallclock = mapper.rtp_to_wallclock(VIDEO_SSRC, video_capture_timestamp)
        .expect("Failed to convert video timestamp to wallclock time");
    
    // Calculate offset between streams
    let audio_to_video_offset = mapper.get_sync_offset(AUDIO_SSRC, VIDEO_SSRC)
        .expect("Failed to calculate sync offset");
    
    info!("Audio timestamp: {}, maps to wallclock {:?}", 
          audio_capture_timestamp, audio_wallclock);
    
    info!("Video timestamp: {}, maps to wallclock {:?}", 
          video_capture_timestamp, video_wallclock);
    
    // The offset will be approximately 100ms
    info!("Calculated synchronization offset: {:.2}ms", audio_to_video_offset);
    
    // Now let's fix the lip sync by applying the calculated offset
    info!("Fixing lip sync by applying offset correction");
    
    // When playing the video frame, we need to delay it by the offset amount
    let play_immediately = false;
    
    if play_immediately {
        info!("Playing video frame immediately (uncorrected)");
    } else {
        // Convert the timestamp to correct for lip sync
        let corrected_video_ts = mapper.map_timestamp(
            AUDIO_SSRC, VIDEO_SSRC, audio_capture_timestamp
        ).expect("Failed to map timestamp");
        
        info!("Original video timestamp: {}", video_capture_timestamp);
        info!("Corrected video timestamp: {}", corrected_video_ts);
        
        // Calculate the difference in milliseconds
        let diff_ms = (corrected_video_ts as i64 - video_capture_timestamp as i64) as f64 / 
                      (VIDEO_CLOCK_RATE as f64 / 1000.0);
                      
        info!("Timestamp correction: {:.2}ms", diff_ms);
    }
    
    // Now let's simulate clock drift between the two streams
    info!("\nSimulating clock drift between audio and video streams");
    
    // Create reference timestamps for before and after the drift period
    let start_ntp = NtpTimestamp::now();
    let start_audio_rtp = 0;
    let start_video_rtp = 0;
    
    // Update timing info with starting points
    mapper.update_from_sr(AUDIO_SSRC, start_ntp, start_audio_rtp);
    mapper.update_from_sr(VIDEO_SSRC, start_ntp, start_video_rtp);
    
    // Wait 2 seconds (simulating time passing)
    info!("Waiting 2 seconds to simulate time passing...");
    time::sleep(Duration::from_secs(2)).await;
    
    // End timestamps - video clock runs 0.5% faster (5000 ppm drift)
    let end_ntp = NtpTimestamp::now();
    let end_audio_rtp = AUDIO_CLOCK_RATE * 2; // 2 seconds worth of samples
    
    // Video clock is running 0.5% fast
    let end_video_rtp = (VIDEO_CLOCK_RATE as f64 * 2.0 * 1.005) as u32;
    
    // Update with ending timestamps
    mapper.update_from_sr(AUDIO_SSRC, end_ntp, end_audio_rtp);
    mapper.update_from_sr(VIDEO_SSRC, end_ntp, end_video_rtp);
    
    // Update mapping with these new reference points
    mapper.map_streams(
        AUDIO_SSRC, VIDEO_SSRC, 
        end_audio_rtp, end_video_rtp, 
        end_ntp
    );
    
    // Get measured drift
    let drift = mapper.get_drift(AUDIO_SSRC, VIDEO_SSRC)
        .expect("Failed to calculate drift");
    
    info!("Measured clock drift between audio and video: {:.2} PPM", drift);
    
    // Calculate synchronization offset after drift
    let offset_after_drift = mapper.get_sync_offset(AUDIO_SSRC, VIDEO_SSRC)
        .expect("Failed to calculate sync offset after drift");
    
    info!("Synchronization offset after drift: {:.2}ms", offset_after_drift);
    
    // Now we'll convert a timestamp with drift compensation
    let audio_ts = end_audio_rtp + AUDIO_CLOCK_RATE; // 1 more second
    let video_ts_with_drift = mapper.map_timestamp(
        AUDIO_SSRC, VIDEO_SSRC, audio_ts
    ).expect("Failed to map timestamp with drift");
    
    // Calculate expected video timestamp without drift compensation
    let video_ts_without_drift = end_video_rtp + VIDEO_CLOCK_RATE;
    
    info!("Audio timestamp: {} (3 seconds)", audio_ts);
    info!("Video timestamp with drift compensation: {}", video_ts_with_drift);
    info!("Video timestamp without compensation: {}", video_ts_without_drift);
    
    // Calculate the difference in milliseconds
    let comp_diff_ms = (video_ts_with_drift as i64 - video_ts_without_drift as i64) as f64 / 
                     (VIDEO_CLOCK_RATE as f64 / 1000.0);
                     
    info!("Drift compensation difference: {:.2}ms", comp_diff_ms);
    
    // Demonstrate using MediaSync interface
    info!("\nDemonstrating MediaSync interface");
    let mut media_sync = MediaSync::new();
    
    // Register our streams
    media_sync.register_stream(AUDIO_SSRC, AUDIO_CLOCK_RATE);
    media_sync.register_stream(VIDEO_SSRC, VIDEO_CLOCK_RATE);
    
    // Set audio as reference stream (typical for lip sync)
    media_sync.set_reference_stream(AUDIO_SSRC);
    
    // Update with SR info
    media_sync.update_from_sr(AUDIO_SSRC, start_ntp, start_audio_rtp);
    media_sync.update_from_sr(VIDEO_SSRC, start_ntp, start_video_rtp);
    
    // Check if streams are synchronized
    let sync_status = media_sync.are_synchronized(AUDIO_SSRC, VIDEO_SSRC, 50.0);
    info!("Streams synchronized within 50ms tolerance: {}", sync_status);
    
    // Convert a timestamp from audio to video
    let video_from_audio = media_sync.convert_timestamp(
        AUDIO_SSRC, VIDEO_SSRC, AUDIO_CLOCK_RATE
    );
    
    if let Some(video_ts) = video_from_audio {
        info!("1s audio timestamp ({}) maps to video timestamp: {}", 
              AUDIO_CLOCK_RATE, video_ts);
    }
    
    // Convert an RTP timestamp to NTP wall clock time
    let rtp_1sec = AUDIO_CLOCK_RATE;
    let ntp_from_rtp = media_sync.rtp_to_ntp(AUDIO_SSRC, rtp_1sec);
    
    if let Some(ntp) = ntp_from_rtp {
        info!("1s audio timestamp ({}) maps to NTP timestamp: {:?}", 
              rtp_1sec, ntp);
    }
    
    info!("Media synchronization example completed successfully");
    
    Ok(())
} 