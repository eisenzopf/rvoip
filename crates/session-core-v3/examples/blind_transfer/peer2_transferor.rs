//! Bob - Truly simple sequential SIP peer
//! 
//! This demonstrates the sequential SimplePeer API - just 35 lines!

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
        sip_port: 5061,
        local_ip: "127.0.0.1".parse()?,
        bind_addr: "127.0.0.1:5061".parse()?,
        local_uri: "sip:bob@127.0.0.1:5061".to_string(),
        ..Default::default()
    };

    let mut bob = SimplePeer::with_config("bob", config).await?;
    println!("[BOB] Listening...");

    // Wait for incoming call
    let (call_id, from) = bob.wait_for_incoming_call().await?;
    println!("[BOB] Call from {}", from);
    
    bob.accept(&call_id).await?;
    let (sent, rcv) = bob.exchange_audio(&call_id, Duration::from_secs(5), |i| generate_tone(880.0, i)).await?;
    save_wav("bob_sent.wav", &sent)?;
    save_wav("bob_received.wav", &rcv)?;

    // Transfer to Charlie
    println!("[BOB] Initiating transfer...");
    bob.send_refer(&call_id, "sip:charlie@127.0.0.1:5062").await?;
    bob.hangup(&call_id).await?;
    
    println!("[BOB] ✅ Completed!");
    bob.shutdown(Duration::from_secs(5)).await?;
    Ok(())
}

fn save_wav(name: &str, samples: &[i16]) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all("output")?;
    let spec = hound::WavSpec { channels: 1, sample_rate: 8000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut writer = hound::WavWriter::create(format!("output/{}", name), spec)?;
    for &sample in samples { writer.write_sample(sample)?; }
    writer.finalize()?;
    println!("[BOB] 📁 Saved output/{}", name);
    Ok(())
}
