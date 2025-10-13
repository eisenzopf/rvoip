//! Alice - Simple blocking handler SIP peer that handles transfers
//! 
//! This demonstrates the new simplified blocking SimplePeer API.
//! Main function is under 50 lines with simple, linear code.

use rvoip_session_core_v3::api::simple::{SimplePeer, Config};
use tokio::time::{sleep, Duration};

// Audio generation helper
fn generate_tone(freq: f32, frame_num: usize) -> Vec<i16> {
    (0..160).map(|j| {
        (0.3 * (2.0 * std::f32::consts::PI * freq * (frame_num * 160 + j) as f32 / 8000.0).sin() * 32767.0) as i16
    }).collect()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("rvoip_session_core_v3=info".parse()?)
        )
        .init();

    println!("\n[ALICE] Starting - Simple blocking handler SIP peer");

    let config = Config {
        sip_port: 5060,
        local_ip: "127.0.0.1".parse()?,
        bind_addr: "127.0.0.1:5060".parse()?,
        local_uri: "sip:alice@127.0.0.1:5060".to_string(),
        ..Default::default()
    };

    let mut alice = SimplePeer::with_config("alice", config).await?;

    // Register transfer handler - clean and simple!
    alice.on_refer_received(|event, controller| async move {
        if let rvoip_session_core_v3::api::simple::Event::ReferReceived { call_id, refer_to, .. } = event {
            println!("[ALICE] 🔄 Got REFER to: {}", refer_to);
            
            // Per RFC 3515: Alice (transferee) should NOT hangup - Bob will send BYE
            // Alice should immediately call Charlie
            println!("[ALICE] 📞 Calling Charlie...");
            
            if let Ok(charlie_call_id) = controller.call(&refer_to).await {
                println!("[ALICE] ✅ Connected to Charlie!");
                
                // Exchange audio with Charlie (longer duration for better reception)
                if let Ok((sent, received)) = controller.exchange_audio(&charlie_call_id, Duration::from_secs(5), |i| generate_tone(440.0, i)).await {
                    println!("[ALICE] 📁 Saving Charlie audio ({} sent, {} received samples)", sent.len(), received.len());
                    save_wav("output/alice_to_charlie_sent.wav", &sent).ok();
                    save_wav("output/alice_from_charlie_received.wav", &received).ok();
                }
                
                controller.hangup(&charlie_call_id).await.ok();
            } else {
                println!("[ALICE] ❌ Failed to call Charlie");
            }
        }
    }).await;
    
    // Register call answered handler to track Bob audio
    alice.on_call_answered(|event, controller| async move {
        if let rvoip_session_core_v3::api::simple::Event::CallAnswered { call_id, .. } = event {
            println!("[ALICE] ✅ Call with Bob answered");
            
            // Exchange audio with Bob (longer duration for better reception)
            if let Ok((sent, received)) = controller.exchange_audio(&call_id, Duration::from_secs(5), |i| generate_tone(440.0, i)).await {
                println!("[ALICE] 📁 Saving Bob audio ({} sent, {} received samples)", sent.len(), received.len());
                save_wav("output/alice_to_bob_sent.wav", &sent).ok();
                save_wav("output/alice_from_bob_received.wav", &received).ok();
            }
        }
    }).await;

    sleep(Duration::from_secs(3)).await; // Wait for other peers

    println!("[ALICE] 📞 Calling Bob...");
    let _bob_call_id = alice.call("sip:bob@127.0.0.1:5061").await?;

    // Wait for transfer to complete (callbacks handle everything)
    loop {
        sleep(Duration::from_secs(1)).await;
    }
    
    println!("[ALICE] ✅ Completed!");
    Ok(())
}

fn save_wav(path: &str, samples: &[i16]) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all("output")?;
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 8000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for &sample in samples {
        writer.write_sample(sample)?;
    }
    writer.finalize()?;
    println!("[ALICE] 📁 Saved {}", path);
    Ok(())
}
