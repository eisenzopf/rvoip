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
                let _call_handle = controller.accept(&call_id).await.ok();
                println!("[BOB] ✅ Call accepted");
                
                // Wait a moment to simulate conversation, then initiate transfer
                sleep(Duration::from_secs(2)).await;
                
                // Initiate transfer to Charlie
                println!("[BOB] 🔄 Initiating transfer to Charlie...");
                controller.send_refer(&call_id, "sip:charlie@127.0.0.1:5062").await.ok();
                
                // Wait a moment, then terminate call (per SIP REFER semantics)
                sleep(Duration::from_secs(1)).await;
                println!("[BOB] 👋 Terminating call after REFER...");
                controller.hangup(&call_id).await.ok();
                
                // Save some dummy audio for the test
                if let Ok(mut samples) = audio_samples.lock() {
                    for i in 0..50 {
                        let new_samples: Vec<i16> = (0..160).map(|j| {
                            (0.3 * (2.0 * std::f32::consts::PI * 880.0 * (i * 160 + j) as f32 / 8000.0).sin() * 32767.0) as i16
                        }).collect();
                        samples.extend(new_samples);
                    }
                }
            }
        }
    }).await;

    println!("[BOB] ✅ Listening on port 5061...");

    // Wait for calls (callbacks handle everything automatically)
    sleep(Duration::from_secs(15)).await;

    // Save audio
    if let Ok(samples) = audio_samples.lock() {
        if !samples.is_empty() {
            std::fs::create_dir_all("output")?;
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: 8000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            
            let mut writer = hound::WavWriter::create("output/bob_sent.wav", spec)?;
            for sample in samples.iter() {
                writer.write_sample(*sample)?;
            }
            writer.finalize()?;
            println!("[BOB] 📁 Saved audio to bob_sent.wav");
        }
    }

    println!("[BOB] ✅ Transfer initiated successfully!");
    Ok(())
}