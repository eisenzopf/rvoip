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
                
                // Accept the transferred call
                let _call_handle = controller.accept(&call_id).await.ok();
                println!("[CHARLIE] ✅ Transfer call accepted");
                
                // Generate some audio for the test
                if let Ok(mut samples) = audio_samples.lock() {
                    for i in 0..50 {
                        let new_samples: Vec<i16> = (0..160).map(|j| {
                            (0.3 * (2.0 * std::f32::consts::PI * 660.0 * (i * 160 + j) as f32 / 8000.0).sin() * 32767.0) as i16
                        }).collect();
                        samples.extend(new_samples);
                    }
                }
                
                // Keep the call active for a while
                sleep(Duration::from_secs(3)).await;
            }
        }
    }).await;

    println!("[CHARLIE] ✅ Listening on port 5062...");

    // Wait for transferred calls (callbacks handle everything automatically)
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
            
            let mut writer = hound::WavWriter::create("output/charlie_received.wav", spec)?;
            for sample in samples.iter() {
                writer.write_sample(*sample)?;
            }
            writer.finalize()?;
            println!("[CHARLIE] 📁 Saved audio to charlie_received.wav");
        }
    }

    println!("[CHARLIE] ✅ Transfer target example completed!");
    Ok(())
}