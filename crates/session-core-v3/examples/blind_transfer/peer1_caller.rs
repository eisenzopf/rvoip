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
    
    // Track call IDs to distinguish Bob from Charlie
    let bob_call_id = Arc::new(Mutex::new(None::<String>));
    let charlie_call_id = Arc::new(Mutex::new(None::<String>));
    let handled_calls = Arc::new(Mutex::new(std::collections::HashSet::new()));

    // Register transfer handler
    alice.on_refer_received(|event, controller| async move {
        if let rvoip_session_core_v3::api::simple::Event::ReferReceived { call_id, refer_to, .. } = event {
            println!("[ALICE] 🔄 Got REFER to: {}", refer_to);
            
            // Per RFC 3515: Alice should call Charlie (Bob will send BYE)
            println!("[ALICE] 📞 Calling Charlie...");
            controller.call(&refer_to).await.ok();
        }
    }).await;
    
    // Register call answered handler - handles both Bob and Charlie
    let bob_call_id_for_answered = bob_call_id.clone();
    let charlie_call_id_for_answered = charlie_call_id.clone();
    let handled_calls_clone = handled_calls.clone();
    alice.on_call_answered(move |event, controller| {
        let bob_call_id = bob_call_id_for_answered.clone();
        let charlie_call_id = charlie_call_id_for_answered.clone();
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
                
                // Check if this is the first call (Bob) or second call (Charlie)
                let has_bob = bob_call_id.lock().unwrap().is_some();
                
                if !has_bob {
                    // First call - this is Bob
                    println!("[ALICE] ✅ Call with Bob answered");
                    bob_call_id.lock().unwrap().replace(call_id.0.clone());
                    
                    // Exchange audio with Bob
                    if let Ok((sent, received)) = controller.exchange_audio(&call_id, Duration::from_secs(5), |i| generate_tone(440.0, i)).await {
                        println!("[ALICE] 📁 Saving Bob audio ({} sent, {} received samples)", sent.len(), received.len());
                        save_wav("output/alice_to_bob_sent.wav", &sent).ok();
                        save_wav("output/alice_from_bob_received.wav", &received).ok();
                    }
                } else {
                    // Second call - this is Charlie
                    println!("[ALICE] ✅ Call with Charlie answered");
                    charlie_call_id.lock().unwrap().replace(call_id.0.clone());
                    
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
    
    // Register call ended handler to exit when Charlie call ends (not Bob)
    let charlie_call_id_for_ended = charlie_call_id.clone();
    alice.on_call_ended(move |event, _controller| {
        let charlie_call_id = charlie_call_id_for_ended.clone();
        async move {
            if let rvoip_session_core_v3::api::simple::Event::CallEnded { call_id, reason } = event {
                // Check if this is the Charlie call
                let is_charlie_call = charlie_call_id.lock().unwrap()
                    .as_ref()
                    .map(|id| id == &call_id.0)
                    .unwrap_or(false);
                
                if is_charlie_call {
                    println!("[ALICE] 📞 Charlie call ended: {} ({})", call_id.0, reason);
                    // Exit after Charlie call ends
                    sleep(Duration::from_secs(1)).await;
                    println!("[ALICE] ✅ Completed!");
                    std::process::exit(0);
                } else {
                    println!("[ALICE] 📞 Bob call ended: {} ({})", call_id.0, reason);
                    // Don't exit - Alice still needs to talk to Charlie
                }
            }
        }
    }).await;

    sleep(Duration::from_secs(3)).await; // Wait for other peers

    println!("[ALICE] 📞 Calling Bob...");
    let _bob_call_id = alice.call("sip:bob@127.0.0.1:5061").await?;

    // Wait for transfer to complete (callbacks handle everything and exit)
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
