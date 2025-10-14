//! Charlie - Truly simple sequential SIP peer
//! 
//! This demonstrates the sequential SimplePeer API - just 30 lines!

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
        sip_port: 5062,
        local_ip: "127.0.0.1".parse()?,
        bind_addr: "127.0.0.1:5062".parse()?,
        local_uri: "sip:charlie@127.0.0.1:5062".to_string(),
        ..Default::default()
    };

    let mut charlie = SimplePeer::with_config("charlie", config).await?;
    println!("[CHARLIE] Listening...");

    // Wait for transferred call
    let (call_id, from) = charlie.wait_for_incoming_call().await?;
    println!("[CHARLIE] Transferred call from {}", from);
    
    charlie.accept(&call_id).await?;
    let (sent, rcv) = charlie.exchange_audio(&call_id, Duration::from_secs(5), |i| generate_tone(660.0, i)).await?;
    save_wav("charlie_sent.wav", &sent)?;
    save_wav("charlie_received.wav", &rcv)?;

    // Wait for Alice to hangup (receive CallEnded event)
    charlie.wait_for_call_ended().await?;
    
    println!("[CHARLIE] ✅ Completed!");
    charlie.shutdown(Duration::from_secs(1)).await?;
    Ok(())
}

fn save_wav(name: &str, samples: &[i16]) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all("output")?;
    let spec = hound::WavSpec { channels: 1, sample_rate: 8000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut writer = hound::WavWriter::create(format!("output/{}", name), spec)?;
    for &sample in samples { writer.write_sample(sample)?; }
    writer.finalize()?;
    println!("[CHARLIE] 📁 Saved output/{}", name);
    Ok(())
}
