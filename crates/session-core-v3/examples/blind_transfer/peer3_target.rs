//! Charlie - Simple SIP peer that receives transferred calls
//! 
//! This demonstrates the simplified blocking SimplePeer API for the transfer target role.

use rvoip_session_core_v3::api::simple::{SimplePeer, Config};
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

    // Register incoming call handler - clean and simple!
    charlie.on_incoming_call(|event, controller| async move {
        if let rvoip_session_core_v3::api::simple::Event::IncomingCall { call_id, from, .. } = event {
            println!("[CHARLIE] 📞 Transferred call from: {}", from);
            
            controller.accept(&call_id).await.ok();
            println!("[CHARLIE] ✅ Call accepted");
            
            // Exchange real audio with Alice (longer duration for better reception)
            if let Ok((sent, received)) = controller.exchange_audio(&call_id, Duration::from_secs(5), |i| generate_tone(660.0, i)).await {
                println!("[CHARLIE] 📁 Saving audio ({} sent, {} received samples)", sent.len(), received.len());
                save_wav("output/charlie_sent.wav", &sent).ok();
                save_wav("output/charlie_received.wav", &received).ok();
            }
            
            println!("[CHARLIE] ✅ Transfer call complete!");
        }
    }).await;
    
    // Register call ended handler to exit when call ends
    charlie.on_call_ended(|event, _controller| async move {
        if let rvoip_session_core_v3::api::simple::Event::CallEnded { call_id, reason } = event {
            println!("[CHARLIE] 📞 Call ended: {} ({})", call_id.0, reason);
            // Exit after call ends
            tokio::time::sleep(Duration::from_secs(1)).await;
            println!("[CHARLIE] ✅ Completed!");
            std::process::exit(0);
        }
    }).await;

    println!("[CHARLIE] ✅ Listening on port 5062...");
    // Wait for transferred calls (callbacks handle everything and exit)
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    
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
