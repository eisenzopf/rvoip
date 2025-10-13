//! Bob - Simple SIP peer that receives call and initiates transfer
//! 
//! This demonstrates the simplified blocking SimplePeer API for the transferor role.

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

    println!("\n[BOB] Starting - Will receive call and initiate transfer");

    let config = Config {
        sip_port: 5061,
        local_ip: "127.0.0.1".parse()?,
        bind_addr: "127.0.0.1:5061".parse()?,
        local_uri: "sip:bob@127.0.0.1:5061".to_string(),
        ..Default::default()
    };

    let mut bob = SimplePeer::with_config("bob", config).await?;

    // Register incoming call handler - simple and direct!
    bob.on_incoming_call(|call_id, from, peer| async move {
        println!("[BOB] 📞 Incoming call from: {}", from);
        
        peer.accept(&call_id).await?;
        println!("[BOB] ✅ Call accepted");
        
        // Exchange real audio with Alice
        let (sent, received) = peer.exchange_audio(&call_id, Duration::from_secs(2), |i| generate_tone(880.0, i)).await?;
        
        println!("[BOB] 📁 Saving audio ({} sent, {} received samples)", sent.len(), received.len());
        save_wav("output/bob_sent.wav", &sent)?;
        save_wav("output/bob_received.wav", &received)?;
        
        // Transfer to Charlie
        println!("[BOB] 🔄 Initiating transfer...");
        peer.send_refer(&call_id, "sip:charlie@127.0.0.1:5062").await?;
        peer.hangup(&call_id).await?;
        
        println!("[BOB] ✅ Transfer complete!");
        Ok(())
    });

    println!("[BOB] ✅ Listening on port 5061...");
    bob.run().await?; // Simple event loop!
    
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
    println!("[BOB] 📁 Saved {}", path);
    Ok(())
}
