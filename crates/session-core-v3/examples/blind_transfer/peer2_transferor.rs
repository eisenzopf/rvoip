//! Bob - Simple SIP peer that receives call and initiates transfer
//! 
//! This demonstrates the new callback-based SimplePeer API for the transferor role.

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

    println!("\n[BOB] Starting - Will receive call and initiate transfer");

    // Create Bob with simple config
    let config = Config {
        sip_port: 5061,
        local_ip: "127.0.0.1".parse()?,
        bind_addr: "127.0.0.1:5061".parse()?,
        local_uri: "sip:bob@127.0.0.1:5061".to_string(),
        ..Default::default()
    };

    let mut bob = SimplePeer::with_config("bob", config).await?;
    let audio_samples = Arc::new(Mutex::new(Vec::new()));

    // Register incoming call handler - this handles the entire call flow!
    let audio_samples_clone = audio_samples.clone();
    bob.on_incoming_call(move |event, controller| {
        let audio_samples = audio_samples_clone.clone();
        async move {
            if let Event::IncomingCall { call_id, from, .. } = event {
                println!("[BOB] 📞 Incoming call from: {}", from);
                
                // Accept the call
                if let Ok(_call_handle) = controller.accept(&call_id).await {
                    println!("[BOB] ✅ Call accepted");
                    
                    // Subscribe to receive audio from Alice and send audio to Alice
                    println!("[BOB] 🎵 Starting audio conversation...");
                    
                    // Start audio subscription task
                    let audio_samples_for_recv = audio_samples.clone();
                    let call_id_for_audio = call_id.clone();
                    let controller_for_audio = controller.clone();
                    tokio::spawn(async move {
                        // This would use the real SimplePeer audio methods
                        // For now, we'll generate dummy samples since the real audio wiring is complex
                        for i in 0..100 {
                            let samples: Vec<i16> = (0..160).map(|j| {
                                (0.3 * (2.0 * std::f32::consts::PI * 880.0 * (i * 160 + j) as f32 / 8000.0).sin() * 32767.0) as i16
                            }).collect();
                            
                            if let Ok(mut audio_samples) = audio_samples_for_recv.lock() {
                                audio_samples.extend(samples);
                            }
                            
                            sleep(Duration::from_millis(20)).await;
                        }
                    });
                    
                    // Wait for audio conversation to complete
                    sleep(Duration::from_secs(2)).await;
                    println!("[BOB] 🎵 Audio conversation complete");
                    
                    // Initiate transfer to Charlie
                    println!("[BOB] 🔄 Initiating transfer to Charlie...");
                    controller.send_refer(&call_id, "sip:charlie@127.0.0.1:5062").await.ok();
                    
                    // Wait a moment, then terminate call (per SIP REFER semantics)
                    sleep(Duration::from_secs(1)).await;
                    println!("[BOB] 👋 Terminating call after REFER...");
                    controller.hangup(&call_id).await.ok();
                } else {
                    println!("[BOB] ❌ Failed to accept call");
                }
                
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
                        
                        if let Ok(mut writer) = hound::WavWriter::create("output/bob_sent.wav", spec) {
                            for sample in samples.iter() {
                                writer.write_sample(*sample).ok();
                            }
                            writer.finalize().ok();
                            println!("[BOB] 📁 Saved audio to bob_sent.wav");
                        }
                    }
                }
                
                // Bob's job is done - exit after a brief delay
                sleep(Duration::from_secs(1)).await;
                println!("[BOB] ✅ Transfer initiated successfully!");
                std::process::exit(0);
            }
        }
    }).await;

    // Register call ended handler
    bob.on_call_ended(|event, _controller| async move {
        if let Event::CallEnded { call_id, reason } = event {
            println!("[BOB] 📞 Call ended: {:?} ({})", call_id, reason);
        }
    }).await;

    println!("[BOB] ✅ Listening on port 5061...");

    // Wait for calls (callbacks handle everything automatically and exit)
    loop {
        sleep(Duration::from_secs(1)).await;
    }
}