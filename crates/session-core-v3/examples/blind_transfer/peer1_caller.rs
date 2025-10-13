//! Alice - Simple event-driven SIP peer that handles transfers
//! 
//! This demonstrates the new event-driven SimplePeer API.
//! Main function is under 50 lines and handles all scenarios cleanly.

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

    println!("\n[ALICE] Starting - Simple event-driven SIP peer");

    // Create Alice with simple config
    let config = Config {
        sip_port: 5060,
        local_ip: "127.0.0.1".parse()?,
        bind_addr: "127.0.0.1:5060".parse()?,
        local_uri: "sip:alice@127.0.0.1:5060".to_string(),
        ..Default::default()
    };

    let mut alice = SimplePeer::with_config("alice", config).await?;
    let mut audio_samples = Vec::new();

    // Give other peers time to start
    sleep(Duration::from_secs(3)).await;

    // Call Bob
    println!("[ALICE] 📞 Calling Bob...");
    let mut bob_call = alice.call("sip:bob@127.0.0.1:5061").await?;

    // Simple event loop - this is the entire application logic!
    while let Some(event) = alice.next_event().await {
        match event {
            Event::CallAnswered { call_id, .. } => {
                println!("[ALICE] ✅ Call answered: {:?}", call_id);
                
                // Send some audio to Bob
                for i in 0..100 {
                    let samples: Vec<i16> = (0..160).map(|j| {
                        (0.3 * (2.0 * std::f32::consts::PI * 440.0 * (i * 160 + j) as f32 / 8000.0).sin() * 32767.0) as i16
                    }).collect();
                    
                    bob_call.send_audio(samples.clone()).await?;
                    audio_samples.extend(samples);
                    
                    // Receive audio from Bob
                    if let Ok(received) = bob_call.try_recv_audio() {
                        audio_samples.extend(received);
                    }
                    
                    sleep(Duration::from_millis(20)).await;
                }
            }
            
            Event::ReferReceived { refer_to, .. } => {
                println!("[ALICE] 🔄 Got REFER to: {}", refer_to);
                
                // Handle transfer immediately - this is the entire transfer logic!
                alice.hangup(bob_call.call_id()).await?;
                let mut charlie_call = alice.call(&refer_to).await?;
                
                println!("[ALICE] ✅ Transfer complete! Now talking to Charlie");
                
                // Talk to Charlie
                for i in 0..50 {
                    let samples: Vec<i16> = (0..160).map(|j| {
                        (0.3 * (2.0 * std::f32::consts::PI * 440.0 * (i * 160 + j) as f32 / 8000.0).sin() * 32767.0) as i16
                    }).collect();
                    
                    charlie_call.send_audio(samples.clone()).await?;
                    audio_samples.extend(samples);
                    
                    if let Ok(received) = charlie_call.try_recv_audio() {
                        audio_samples.extend(received);
                    }
                    
                    sleep(Duration::from_millis(20)).await;
                }
                
                alice.hangup(charlie_call.call_id()).await?;
            }
            
            Event::CallEnded { call_id, reason } => {
                println!("[ALICE] 📞 Call ended: {:?} ({})", call_id, reason);
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
        
        let mut writer = hound::WavWriter::create("output/alice_sent.wav", spec)?;
        for sample in audio_samples {
            writer.write_sample(sample)?;
        }
        writer.finalize()?;
        println!("[ALICE] 📁 Saved audio to alice_sent.wav");
    }

    println!("[ALICE] ✅ Simple event-driven example completed!");
    Ok(())
}