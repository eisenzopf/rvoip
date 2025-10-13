//! Charlie - Simple SIP peer that receives transferred calls
//! 
//! This demonstrates the simplified blocking SimplePeer API for the transfer target role.

use rvoip_session_core_v3::api::simple::{SimplePeer, Config, CallId};
use tokio::time::Duration;

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

    println!("\n[CHARLIE] Starting - Will receive transferred call from Alice");

    let config = Config {
        sip_port: 5062,
        local_ip: "127.0.0.1".parse()?,
        bind_addr: "127.0.0.1:5062".parse()?,
        local_uri: "sip:charlie@127.0.0.1:5062".to_string(),
        ..Default::default()
    };

    let mut charlie = SimplePeer::with_config("charlie", config).await?;

    // Register incoming call handler - simple and direct!
    charlie.on_incoming_call(|call_id, from, peer| async move {
        println!("[CHARLIE] 📞 Transferred call from: {}", from);
        
        peer.accept(&call_id).await?;
        println!("[CHARLIE] ✅ Call accepted");
        
        // Exchange real audio with Alice
        let (sent, received) = peer.exchange_audio(&call_id, Duration::from_secs(3), |i| generate_tone(660.0, i)).await?;
        
        println!("[CHARLIE] 📁 Saving audio ({} sent, {} received samples)", sent.len(), received.len());
        save_wav("output/charlie_sent.wav", &sent)?;
        save_wav("output/charlie_received.wav", &received)?;
        
        println!("[CHARLIE] ✅ Transfer call complete!");
        Ok(())
    });

    println!("[CHARLIE] ✅ Listening on port 5062...");
    charlie.run().await?; // Simple event loop!
    
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
    println!("[CHARLIE] 📁 Saved {}", path);
    Ok(())
}
