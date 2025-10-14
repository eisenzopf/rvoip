//! Alice - Truly simple sequential SIP peer
//! 
//! This demonstrates the sequential SimplePeer API - just 40 lines!

use rvoip_session_core_v3::api::simple::{SimplePeer, Config};
use tokio::time::Duration;

fn generate_tone(freq: f32, frame_num: usize) -> Vec<i16> {
    (0..160).map(|j| {
        (0.3 * (2.0 * std::f32::consts::PI * freq * (frame_num * 160 + j) as f32 / 8000.0).sin() * 32767.0) as i16
    }).collect()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env()
            .add_directive("rvoip_session_core_v3=info".parse()?))
        .init();

    let config = Config {
        sip_port: 5060,
        local_ip: "127.0.0.1".parse()?,
        bind_addr: "127.0.0.1:5060".parse()?,
        local_uri: "sip:alice@127.0.0.1:5060".to_string(),
        ..Default::default()
    };

    let mut alice = SimplePeer::with_config("alice", config).await?;
    tokio::time::sleep(Duration::from_secs(3)).await; // Wait for peers

    // Call Bob
    println!("[ALICE] Calling Bob...");
    let bob_id = alice.call("sip:bob@127.0.0.1:5061").await?;
    alice.wait_for_answered(&bob_id).await?;
    
    let (sent, rcv) = alice.exchange_audio(&bob_id, Duration::from_secs(5), |i| generate_tone(440.0, i)).await?;
    save_wav("alice_to_bob_sent.wav", &sent)?;
    save_wav("alice_from_bob_received.wav", &rcv)?;

    // Wait for REFER and complete transfer to Charlie
    if let Some(refer) = alice.wait_for_refer().await? {
        println!("[ALICE] Got REFER to {} (type: {}, txn: {})", 
                 refer.refer_to, refer.transfer_type, refer.transaction_id);
        
        // Use the new complete_blind_transfer helper
        let charlie_id = alice.complete_blind_transfer(&refer).await?;
        alice.wait_for_answered(&charlie_id).await?;
        
        println!("[ALICE] Now talking to Charlie (post-transfer)...");
        let (sent, rcv) = alice.exchange_audio(&charlie_id, Duration::from_secs(5), |i| generate_tone(440.0, i)).await?;
        save_wav("alice_to_charlie_sent.wav", &sent)?;
        save_wav("alice_from_charlie_received.wav", &rcv)?;
        
        alice.hangup(&charlie_id).await?;
    }

    println!("[ALICE] ✅ Completed!");
    alice.shutdown(Duration::from_secs(5)).await?;
    Ok(())
}

fn save_wav(name: &str, samples: &[i16]) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all("output")?;
    let spec = hound::WavSpec { channels: 1, sample_rate: 8000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut writer = hound::WavWriter::create(format!("output/{}", name), spec)?;
    for &sample in samples { writer.write_sample(sample)?; }
    writer.finalize()?;
    println!("[ALICE] 📁 Saved output/{}", name);
    Ok(())
}
