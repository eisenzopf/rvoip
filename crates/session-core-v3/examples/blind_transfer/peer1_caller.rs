//! Alice - Simple callback-based SIP peer that handles transfers
//! 
//! This demonstrates the callback-based SimplePeer API with proper SIP flow.

use rvoip_session_core_v3::api::simple::{SimplePeer, Config};
use tokio::time::{sleep, Duration};
use std::sync::{Arc, Mutex};

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

    println!("\n[ALICE] Starting - Simple callback-based SIP peer");

    let config = Config {
        sip_port: 5060,
        local_ip: "127.0.0.1".parse()?,
        bind_addr: "127.0.0.1:5060".parse()?,
        local_uri: "sip:alice@127.0.0.1:5060".to_string(),
        ..Default::default()
    };

    let mut alice = SimplePeer::with_config("alice", config).await?;
    
    // Track which call we're on and prevent duplicate handling
    let is_bob_call = Arc::new(Mutex::new(true));
    let handled_calls = Arc::new(Mutex::new(std::collections::HashSet::new()));

    // Register transfer handler
    let is_bob_call_for_refer = is_bob_call.clone();
    alice.on_refer_received(move |event, controller| {
        let is_bob_call = is_bob_call_for_refer.clone();
        async move {
            if let rvoip_session_core_v3::api::simple::Event::ReferReceived { call_id, refer_to, .. } = event {
                println!("[ALICE] 🔄 Got REFER to: {}", refer_to);
                
                // Mark that the next CallAnswered will be for Charlie
                if let Ok(mut is_bob) = is_bob_call.lock() {
                    *is_bob = false;
                }
                
                // Per RFC 3515: Alice should call Charlie (Bob will send BYE)
                println!("[ALICE] 📞 Calling Charlie...");
                controller.call(&refer_to).await.ok();
            }
        }
    }).await;
    
    // Register call answered handler - handles both Bob and Charlie
    let is_bob_call_for_answered = is_bob_call.clone();
    let handled_calls_clone = handled_calls.clone();
    alice.on_call_answered(move |event, controller| {
        let is_bob_call = is_bob_call_for_answered.clone();
        let handled_calls = handled_calls_clone.clone();
        async move {
            if let rvoip_session_core_v3::api::simple::Event::CallAnswered { call_id, .. } = event {
                // Prevent duplicate handling of the same call
                {
                    let mut handled = handled_calls.lock().unwrap();
                    if !handled.insert(call_id.0.clone()) {
                        println!("[ALICE] ⚠️ Ignoring duplicate CallAnswered for {}", call_id.0);
                        return;
                    }
                }
                
                let is_bob = is_bob_call.lock().map(|guard| *guard).unwrap_or(true);
                
                if is_bob {
                    println!("[ALICE] ✅ Call with Bob answered");
                    
                    // Exchange audio with Bob
                    if let Ok((sent, received)) = controller.exchange_audio(&call_id, Duration::from_secs(5), |i| generate_tone(440.0, i)).await {
                        println!("[ALICE] 📁 Saving Bob audio ({} sent, {} received samples)", sent.len(), received.len());
                        save_wav("output/alice_to_bob_sent.wav", &sent).ok();
                        save_wav("output/alice_from_bob_received.wav", &received).ok();
                    }
                } else {
                    println!("[ALICE] ✅ Call with Charlie answered");
                    
                    // Exchange audio with Charlie  
                    if let Ok((sent, received)) = controller.exchange_audio(&call_id, Duration::from_secs(5), |i| generate_tone(440.0, i)).await {
                        println!("[ALICE] 📁 Saving Charlie audio ({} sent, {} received samples)", sent.len(), received.len());
                        save_wav("output/alice_to_charlie_sent.wav", &sent).ok();
                        save_wav("output/alice_from_charlie_received.wav", &received).ok();
                    }
                    
                    controller.hangup(&call_id).await.ok();
                }
            }
        }
    }).await;

    sleep(Duration::from_secs(3)).await; // Wait for other peers

    println!("[ALICE] 📞 Calling Bob...");
    let _bob_call_id = alice.call("sip:bob@127.0.0.1:5061").await?;

    // Wait for transfer to complete
    loop {
        sleep(Duration::from_secs(1)).await;
    }
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
