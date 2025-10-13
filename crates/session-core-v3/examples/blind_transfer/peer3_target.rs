//! Charlie - Simple SIP peer that receives transferred calls
//! 
//! This demonstrates the new callback-based SimplePeer API for the transfer target role.

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

    println!("\n[CHARLIE] Starting - Will receive transferred call from Alice");

    // Create Charlie with simple config
    let config = Config {
        sip_port: 5062,
        local_ip: "127.0.0.1".parse()?,
        bind_addr: "127.0.0.1:5062".parse()?,
        local_uri: "sip:charlie@127.0.0.1:5062".to_string(),
        ..Default::default()
    };

    let mut charlie = SimplePeer::with_config("charlie", config).await?;
    let audio_samples = Arc::new(Mutex::new(Vec::new()));

    // Register incoming call handler - this handles transferred calls!
    let audio_samples_clone = audio_samples.clone();
    charlie.on_incoming_call(move |event, controller| {
        let audio_samples = audio_samples_clone.clone();
        async move {
            if let Event::IncomingCall { call_id, from, .. } = event {
                println!("[CHARLIE] 📞 Transferred call from: {}", from);
                
                // Accept the transferred call and get handle for real audio
                if let Ok(mut call_handle) = controller.accept(&call_id).await {
                    println!("[CHARLIE] ✅ Transfer call accepted");
                    
                    // Send real audio through CallHandle and collect received audio
                    println!("[CHARLIE] 🎵 Starting audio conversation with Alice...");
                    for i in 0..50 { // About 1 second of audio
                        // Generate and send audio through CallHandle
                        let samples: Vec<i16> = (0..160).map(|j| {
                            (0.3 * (2.0 * std::f32::consts::PI * 660.0 * (i * 160 + j) as f32 / 8000.0).sin() * 32767.0) as i16
                        }).collect();
                        
                        // Send real audio through CallHandle
                        if call_handle.send_audio(samples.clone()).await.is_ok() {
                            if let Ok(mut audio_samples) = audio_samples.lock() {
                                audio_samples.extend(samples);
                            }
                        }
                        
                        // Receive real audio from Alice through CallHandle
                        if let Ok(received_samples) = call_handle.try_recv_audio() {
                            if let Ok(mut audio_samples) = audio_samples.lock() {
                                audio_samples.extend(received_samples);
                            }
                        }
                        
                        sleep(Duration::from_millis(20)).await;
                    }
                    println!("[CHARLIE] 🎵 Audio conversation complete");
                } else {
                    println!("[CHARLIE] ❌ Failed to accept transferred call");
                }
            }
        }
    }).await;

    // Register call answered handler (for when Alice calls Charlie)
    charlie.on_call_answered(|event, _controller| async move {
        if let Event::CallAnswered { call_id, .. } = event {
            println!("[CHARLIE] ✅ Call with Alice established: {:?}", call_id);
        }
    }).await;

    // Register call ended handler
    let audio_samples_for_ended = audio_samples.clone();
    charlie.on_call_ended(move |event, _controller| {
        let audio_samples = audio_samples_for_ended.clone();
        async move {
            if let Event::CallEnded { call_id, reason } = event {
                println!("[CHARLIE] 📞 Call ended: {:?} ({})", call_id, reason);
                
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
                        
                        if let Ok(mut writer) = hound::WavWriter::create("output/charlie_received.wav", spec) {
                            for sample in samples.iter() {
                                writer.write_sample(*sample).ok();
                            }
                            writer.finalize().ok();
                            println!("[CHARLIE] 📁 Saved audio to charlie_received.wav");
                        }
                    }
                }
                
                // Charlie's job is done when Alice hangs up
                sleep(Duration::from_secs(1)).await;
                println!("[CHARLIE] ✅ Transfer target example completed!");
                std::process::exit(0);
            }
        }
    }).await;

    println!("[CHARLIE] ✅ Listening on port 5062...");

    // Wait for transferred calls (callbacks handle everything automatically and exit)
    loop {
        sleep(Duration::from_secs(1)).await;
    }
}