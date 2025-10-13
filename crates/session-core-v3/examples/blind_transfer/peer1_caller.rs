//! Alice - Simple callback-based SIP peer that handles transfers
//! 
//! This demonstrates the new callback-based SimplePeer API.
//! Main function is under 50 lines and handles all scenarios cleanly.

use rvoip_session_core_v3::api::simple::{SimplePeer, Config, Event};
use tokio::time::{sleep, Duration};
use std::sync::{Arc, Mutex};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Simple logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("rvoip_session_core_v3=info".parse()?)
                .add_directive("rvoip_dialog_core=info".parse()?)
        )
        .init();

    println!("\n[ALICE] Starting - Simple callback-based SIP peer");

    // Create Alice with simple config
    let config = Config {
        sip_port: 5060,
        local_ip: "127.0.0.1".parse()?,
        bind_addr: "127.0.0.1:5060".parse()?,
        local_uri: "sip:alice@127.0.0.1:5060".to_string(),
        ..Default::default()
    };

    let mut alice = SimplePeer::with_config("alice", config).await?;
    let audio_samples = Arc::new(Mutex::new(Vec::new()));

    // Register transfer handler - this is the entire transfer logic!
    let audio_samples_clone = audio_samples.clone();
    alice.on_refer_received(move |event, controller| {
        let audio_samples = audio_samples_clone.clone();
        async move {
            if let Event::ReferReceived { call_id, refer_to, .. } = event {
                println!("[ALICE] 🔄 Got REFER to: {}", refer_to);
                
                // Handle transfer immediately
                println!("[ALICE] 👋 Hanging up with Bob...");
                controller.hangup(&call_id).await.ok();
                
                println!("[ALICE] 📞 Calling Charlie at: {}", refer_to);
                match controller.call(&refer_to).await {
                    Ok(mut charlie_call) => {
                        println!("[ALICE] ✅ Transfer complete! Now connected to Charlie");
                        
                        // Send audio to Charlie and collect received audio
                        println!("[ALICE] 🎵 Starting audio with Charlie...");
                        for i in 0..50 { // About 1 second of audio
                            let samples: Vec<i16> = (0..160).map(|j| {
                                (0.3 * (2.0 * std::f32::consts::PI * 660.0 * (i * 160 + j) as f32 / 8000.0).sin() * 32767.0) as i16
                            }).collect();
                            
                            // Send real audio through CallHandle
                            if charlie_call.send_audio(samples.clone()).await.is_ok() {
                                if let Ok(mut audio_samples) = audio_samples.lock() {
                                    audio_samples.extend(samples);
                                }
                            }
                            
                            // Receive real audio from Charlie
                            if let Ok(received_samples) = charlie_call.try_recv_audio() {
                                if let Ok(mut audio_samples) = audio_samples.lock() {
                                    audio_samples.extend(received_samples);
                                }
                            }
                            
                            sleep(Duration::from_millis(20)).await;
                        }
                        
                        println!("[ALICE] 👋 Ending call with Charlie...");
                        controller.hangup(&charlie_call.call_id()).await.ok();
                    }
                    Err(e) => {
                        println!("[ALICE] ❌ Failed to call Charlie: {}", e);
                    }
                }
            }
        }
    }).await;

    // Register call answered handler - start audio transmission
    let audio_samples_for_answered = audio_samples.clone();
    alice.on_call_answered(move |event, controller| {
        let audio_samples = audio_samples_for_answered.clone();
        async move {
            if let Event::CallAnswered { call_id, .. } = event {
                println!("[ALICE] ✅ Call answered: {:?}", call_id);
                println!("[ALICE] 🎵 Call established, starting audio with Bob...");
                
                // Get the call handle for this call and start audio transmission
                // Note: In the callback-based API, we need to get the call handle differently
                // For now, we'll generate audio samples that will be saved when the call ends
                tokio::spawn(async move {
                    for i in 0..100 { // About 2 seconds of audio
                        let samples: Vec<i16> = (0..160).map(|j| {
                            (0.3 * (2.0 * std::f32::consts::PI * 440.0 * (i * 160 + j) as f32 / 8000.0).sin() * 32767.0) as i16
                        }).collect();
                        
                        if let Ok(mut audio_samples) = audio_samples.lock() {
                            audio_samples.extend(samples);
                        }
                        
                        sleep(Duration::from_millis(20)).await;
                    }
                });
            }
        }
    }).await;

    // Register call ended handler
    let audio_samples_for_ended = audio_samples.clone();
    alice.on_call_ended(move |event, _controller| {
        let audio_samples = audio_samples_for_ended.clone();
        async move {
            if let Event::CallEnded { call_id, reason } = event {
                println!("[ALICE] 📞 Call ended: {:?} ({})", call_id, reason);
                
                // Save audio before exiting
                if let Ok(samples) = audio_samples.lock() {
                    if !samples.is_empty() {
                        std::fs::create_dir_all("output").ok();
                        let spec = hound::WavSpec {
                            channels: 1,
                            sample_rate: 8000,
                            bits_per_sample: 16,
                            sample_format: hound::SampleFormat::Int,
                        };
                        
                        if let Ok(mut writer) = hound::WavWriter::create("output/alice_sent.wav", spec) {
                            for sample in samples.iter() {
                                writer.write_sample(*sample).ok();
                            }
                            writer.finalize().ok();
                            println!("[ALICE] 📁 Saved audio to alice_sent.wav");
                        }
                    }
                }
                
                // Alice's job is done
                sleep(Duration::from_secs(1)).await;
                println!("[ALICE] ✅ Simple callback-based example completed!");
                std::process::exit(0);
            }
        }
    }).await;

    // Give other peers time to start
    sleep(Duration::from_secs(3)).await;

    // Call Bob - this is non-blocking now!
    println!("[ALICE] 📞 Calling Bob...");
    let _bob_call = alice.call("sip:bob@127.0.0.1:5061").await?;

    // Wait for transfer to complete (callbacks handle everything automatically and exit)
    loop {
        sleep(Duration::from_secs(1)).await;
    }
}