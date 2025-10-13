//! Alice - Simple blocking handler SIP peer that handles transfers
//! 
//! This demonstrates the new simplified blocking SimplePeer API.
//! Main function is under 50 lines with simple, linear code.

use rvoip_session_core_v3::api::simple::{SimplePeer, Config, CallId};
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

    // Register transfer handler - simple and direct!
    alice.on_refer_received(|call_id, refer_to, peer| async move {
        println!("[ALICE] 🔄 Got REFER to: {}", refer_to);
        
        peer.hangup(&call_id).await?;
        println!("[ALICE] 📞 Calling Charlie...");
        
        let charlie_call_id = peer.call(&refer_to).await?;
        let (sent, received) = peer.exchange_audio(&charlie_call_id, Duration::from_secs(3), |i| generate_tone(440.0, i)).await?;
        
        println!("[ALICE] 📁 Saving audio ({} sent, {} received samples)", sent.len(), received.len());
        save_wav("output/alice_sent.wav", &sent)?;
        save_wav("output/alice_received.wav", &received)?;
        
        peer.hangup(&charlie_call_id).await?;
        Ok(())
    });

    sleep(Duration::from_secs(3)).await; // Wait for other peers

    println!("[ALICE] 📞 Calling Bob...");
    let _bob_call_id = alice.call("sip:bob@127.0.0.1:5061").await?;

    alice.run().await?; // Simple event loop!
    
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
