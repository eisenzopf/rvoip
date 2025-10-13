//! Bob - Simple SIP peer that receives call and initiates transfer
//! 
//! This demonstrates the new event-driven SimplePeer API for the transferor role.

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
    let mut audio_samples = Vec::new();
    // Storage for call handle (removed unused variable)

    println!("[BOB] ✅ Listening on port 5061...");

    // Simple event loop
    while let Some(event) = bob.next_event().await {
        match event {
            Event::IncomingCall { call_id, from, .. } => {
                println!("[BOB] 📞 Incoming call from: {}", from);
                
                // Accept the call and get handle
                let mut call_handle = bob.accept(&call_id).await?;
                println!("[BOB] ✅ Call accepted");
                
                // Talk for a few seconds, then transfer
                for i in 0..100 { // About 5 seconds
                    let samples: Vec<i16> = (0..160).map(|j| {
                        (0.3 * (2.0 * std::f32::consts::PI * 880.0 * (i * 160 + j) as f32 / 8000.0).sin() * 32767.0) as i16
                    }).collect();
                    
                    call_handle.send_audio(samples.clone()).await?;
                    audio_samples.extend(samples);
                    
                    // Receive audio from Alice
                    if let Ok(received) = call_handle.try_recv_audio() {
                        audio_samples.extend(received);
                    }
                    
                    sleep(Duration::from_millis(20)).await;
                }
                
                // Initiate transfer to Charlie
                println!("[BOB] 🔄 Initiating transfer to Charlie...");
                bob.send_refer(&call_id, "sip:charlie@127.0.0.1:5062").await?;
                
                // Wait a moment, then terminate call (per SIP REFER semantics)
                sleep(Duration::from_secs(1)).await;
                println!("[BOB] 👋 Terminating call after REFER...");
                bob.hangup(&call_id).await?;
                
                // Call handle used for audio operations
            }
            
            Event::CallEnded { call_id, .. } => {
                println!("[BOB] 📞 Call ended: {:?}", call_id);
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
        
        let mut writer = hound::WavWriter::create("output/bob_sent.wav", spec)?;
        for sample in audio_samples {
            writer.write_sample(sample)?;
        }
        writer.finalize()?;
        println!("[BOB] 📁 Saved audio to bob_sent.wav");
    }

    println!("[BOB] ✅ Transfer initiated successfully!");
    Ok(())
}