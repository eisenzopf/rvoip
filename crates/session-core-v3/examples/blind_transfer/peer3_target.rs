//! Charlie - Simple SIP peer that receives transferred call
//! 
//! This demonstrates the new event-driven SimplePeer API for the transfer target role.

use rvoip_session_core_v3::api::simple::{SimplePeer, Config, Event};
use tokio::time::{sleep, Duration};

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
    let mut audio_samples = Vec::new();

    println!("[CHARLIE] ✅ Listening on port 5062...");

    // Simple event loop
    while let Some(event) = charlie.next_event().await {
        match event {
            Event::IncomingCall { call_id, from, .. } => {
                println!("[CHARLIE] 📞 Incoming transferred call from: {}", from);
                
                // Accept the transferred call
                let mut call_handle = charlie.accept(&call_id).await?;
                println!("[CHARLIE] ✅ Transferred call accepted");
                
                // Talk with Alice after transfer
                for i in 0..100 { // About 5 seconds
                    let samples: Vec<i16> = (0..160).map(|j| {
                        (0.3 * (2.0 * std::f32::consts::PI * 660.0 * (i * 160 + j) as f32 / 8000.0).sin() * 32767.0) as i16
                    }).collect();
                    
                    call_handle.send_audio(samples.clone()).await?;
                    audio_samples.extend(samples);
                    
                    // Receive audio from Alice
                    if let Ok(received) = call_handle.try_recv_audio() {
                        audio_samples.extend(received);
                    }
                    
                    sleep(Duration::from_millis(20)).await;
                }
                
                // End call
                println!("[CHARLIE] 👋 Ending transferred call...");
                charlie.hangup(&call_id).await?;
            }
            
            Event::CallEnded { call_id, .. } => {
                println!("[CHARLIE] 📞 Call ended: {:?}", call_id);
                break;
            }
            
            _ => {} // Ignore other events
        }
    }

    // Save audio
    if !audio_samples.is_empty() {
        std::fs::create_dir_all("output")?;
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 8000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        
        let mut writer = hound::WavWriter::create("output/charlie_sent.wav", spec)?;
        for sample in audio_samples {
            writer.write_sample(sample)?;
        }
        writer.finalize()?;
        println!("[CHARLIE] 📁 Saved audio to charlie_sent.wav");
    }

    println!("[CHARLIE] ✅ Transfer target example completed!");
    Ok(())
}