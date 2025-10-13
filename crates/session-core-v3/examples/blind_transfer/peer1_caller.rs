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
                controller.hangup(&call_id).await.ok();
                println!("[ALICE] 📞 Calling Charlie at: {}", refer_to);
                let _charlie_call = controller.call(&refer_to).await.ok();
                
                println!("[ALICE] ✅ Transfer complete! Now connected to Charlie");
                
                // Generate some audio samples for the test
                if let Ok(mut samples) = audio_samples.lock() {
                    for i in 0..100 {
                        let new_samples: Vec<i16> = (0..160).map(|j| {
                            (0.3 * (2.0 * std::f32::consts::PI * 440.0 * (i * 160 + j) as f32 / 8000.0).sin() * 32767.0) as i16
                        }).collect();
                        samples.extend(new_samples);
                    }
                }
            }
        }
    }).await;

    // Register call answered handler
    alice.on_call_answered(|event, _controller| async move {
        if let Event::CallAnswered { call_id, .. } = event {
            println!("[ALICE] ✅ Call answered: {:?}", call_id);
            println!("[ALICE] 🎵 Call established, waiting for REFER from Bob...");
        }
    }).await;

    // Give other peers time to start
    sleep(Duration::from_secs(3)).await;

    // Call Bob - this is non-blocking now!
    println!("[ALICE] 📞 Calling Bob...");
    let _bob_call = alice.call("sip:bob@127.0.0.1:5061").await?;

    // Wait for transfer to complete
    sleep(Duration::from_secs(10)).await;

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
            
            let mut writer = hound::WavWriter::create("output/alice_sent.wav", spec)?;
            for sample in samples.iter() {
                writer.write_sample(*sample)?;
            }
            writer.finalize()?;
            println!("[ALICE] 📁 Saved audio to alice_sent.wav");
        }
    }

    println!("[ALICE] ✅ Simple callback-based example completed!");
    Ok(())
}