//! Peer-to-Peer SIP Voice Call Example
//!
//! This example demonstrates direct SIP calls between two computers without a server.
//! One computer acts as the receiver (waiting for calls) and the other as the caller.

use clap::{Parser, Subcommand};
use colored::*;
use local_ip_address::local_ip;
use rvoip_sip_client::{SipClient, SipClientBuilder, SipClientEvent, AudioDirection};
use std::io::{self, Write};
use tracing::error;

#[derive(Parser)]
#[command(name = "sip-p2p")]
#[command(about = "Peer-to-peer SIP voice calls between two computers")]
#[command(version = "1.0")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start as receiver - wait for incoming calls
    Receive {
        /// Your name (e.g., "alice")
        #[arg(short, long)]
        name: String,
        
        /// Port to listen on (default: 5060)
        #[arg(short, long, default_value = "5060")]
        port: u16,
    },
    
    /// Start as caller - make a call to another computer
    Call {
        /// Your name (e.g., "bob")
        #[arg(short, long)]
        name: String,
        
        /// Target IP address of the receiver
        #[arg(short, long)]
        target: String,
        
        /// Target port (default: 5060)
        #[arg(short = 'P', long, default_value = "5060")]
        target_port: u16,
        
        /// Your local port (default: 5061)
        #[arg(short, long, default_value = "5061")]
        port: u16,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("sip_client_p2p=info,rvoip_sip_client=info")
        .init();
    
    let cli = Cli::parse();
    
    match cli.command {
        Commands::Receive { name, port } => run_receiver(name, port).await?,
        Commands::Call { name, target, target_port, port } => {
            run_caller(name, target, target_port, port).await?
        }
    }
    
    Ok(())
}

async fn run_receiver(name: String, port: u16) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", "═══════════════════════════════════════════".bright_cyan());
    println!("{}", "🎧 SIP P2P Voice Call - RECEIVER MODE".bright_cyan().bold());
    println!("{}", "═══════════════════════════════════════════".bright_cyan());
    
    // Get local IP
    let local_ip = local_ip()?;
    let sip_address = format!("sip:{}@{}:{}", name, local_ip, port);
    
    println!("\n{} {}", "Your name:".bright_yellow(), name.bright_white());
    println!("{} {}", "Your IP:".bright_yellow(), local_ip.to_string().bright_white());
    println!("{} {}", "Your port:".bright_yellow(), port.to_string().bright_white());
    println!("{} {}", "Your SIP address:".bright_yellow(), sip_address.bright_green().bold());
    
    println!("\n{}", "📢 Tell the caller to use this command:".bright_magenta());
    println!("{} {} {} {} {} {}\n", 
        "cargo run --".bright_black(),
        "call".bright_white(),
        "-n their_name".bright_black(),
        "-t".bright_white(),
        local_ip.to_string().bright_green().bold(),
        format!("-P {}", port).bright_black()
    );
    
    // Create SIP client
    let client = SipClientBuilder::new()
        .sip_identity(sip_address.clone())
        .local_address(format!("{}:{}", local_ip, port).parse()?)
        .build()
        .await?;
    
    println!("{}", "✅ SIP client created".green());
    
    // List audio devices
    list_audio_devices(&client).await?;
    
    // Start the client
    client.start().await?;
    println!("{}", "✅ SIP client started".green());
    
    // Subscribe to events
    let mut events = client.event_iter();
    
    println!("\n{}", "📞 Waiting for incoming calls...".yellow().bold());
    println!("{}", "Press Ctrl+C to quit\n".bright_black());
    show_call_controls();
    
    // Handle events
    while let Some(event) = events.next().await {
        match event {
            SipClientEvent::IncomingCall { call, from, .. } => {
                println!("\n{} {}", "📞 INCOMING CALL FROM:".bright_yellow().bold(), from.bright_white());
                println!("{}", "Auto-answering call...".bright_black());
                
                // Small delay to ensure proper event processing
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                
                match client.answer(&call.id).await {
                    Ok(_) => {
                        println!("{}", "✅ Call answered!".green().bold());
                        show_call_controls();
                    }
                    Err(e) => {
                        error!("Failed to answer call: {}", e);
                    }
                }
            }
            
            SipClientEvent::CallConnected { call_id, codec, .. } => {
                println!("{} {} {}", 
                    "🔊 Call connected with codec:".green(),
                    codec.bright_white(),
                    format!("(Call ID: {})", call_id).bright_black()
                );
                println!("{}", "⏳ Establishing media paths...".bright_black());
            }
            
            SipClientEvent::CallEnded { .. } => {
                println!("\n{}", "📞 Call ended".red());
                println!("\n{}", "📞 Waiting for incoming calls...".yellow().bold());
            }
            
            SipClientEvent::AudioLevelChanged { direction, level, .. } => {
                // Show audio level meters
                if direction == AudioDirection::Input {
                    print!("\r{} {}", "🎤 Mic:".bright_black(), draw_audio_meter(level));
                } else {
                    print!(" {} {}", "🔊 Spk:".bright_black(), draw_audio_meter(level));
                }
                io::stdout().flush().unwrap();
            }
            
            _ => {}
        }
    }
    
    Ok(())
}

async fn run_caller(
    name: String, 
    target: String, 
    target_port: u16,
    port: u16
) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", "═══════════════════════════════════════════".bright_cyan());
    println!("{}", "📞 SIP P2P Voice Call - CALLER MODE".bright_cyan().bold());
    println!("{}", "═══════════════════════════════════════════".bright_cyan());
    
    // Get local IP
    let local_ip = local_ip()?;
    let sip_address = format!("sip:{}@{}:{}", name, local_ip, port);
    let target_address = format!("sip:receiver@{}:{}", target, target_port);
    
    println!("\n{} {}", "Your name:".bright_yellow(), name.bright_white());
    println!("{} {}", "Your SIP address:".bright_yellow(), sip_address.bright_green());
    println!("{} {}", "Target address:".bright_yellow(), target_address.bright_green().bold());
    
    // Create SIP client
    let client = SipClientBuilder::new()
        .sip_identity(sip_address)
        .local_address(format!("{}:{}", local_ip, port).parse()?)
        .build()
        .await?;
    
    println!("{}", "\n✅ SIP client created".green());
    
    // List audio devices
    list_audio_devices(&client).await?;
    
    // Start the client
    client.start().await?;
    println!("{}", "✅ SIP client started".green());
    
    // Make the call
    println!("\n{} {}", "📞 Calling:".yellow(), target_address.bright_white());
    let call = match client.call(&target_address).await {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to make call: {}", e);
            return Err(e.into());
        }
    };
    
    println!("{}", "🔔 Ringing...".yellow().blink());
    
    // Subscribe to events
    let mut events = client.event_iter();
    
    // Wait for answer
    match call.wait_for_answer().await {
        Ok(_) => {
            println!("{}", "\n✅ Call connected!".green().bold());
            show_call_controls();
        }
        Err(e) => {
            error!("Call failed: {}", e);
            return Err(e.into());
        }
    }
    
    // Handle events
    while let Some(event) = events.next().await {
        match event {
            SipClientEvent::CallEnded { .. } => {
                println!("\n{}", "📞 Call ended".red());
                break;
            }
            
            SipClientEvent::AudioLevelChanged { direction, level, .. } => {
                // Show audio level meters
                if direction == AudioDirection::Input {
                    print!("\r{} {}", "🎤 Mic:".bright_black(), draw_audio_meter(level));
                } else {
                    print!(" {} {}", "🔊 Spk:".bright_black(), draw_audio_meter(level));
                }
                io::stdout().flush().unwrap();
            }
            
            _ => {}
        }
    }
    
    Ok(())
}


fn show_call_controls() {
    println!("\n{}", "═══════════════════════════════════════════".bright_black());
    println!("{}", "CALL STATUS:".bright_white().bold());
    println!("{}", "  🎤 Audio level meters will appear here during call".bright_black());
    println!("{}", "  📞 Use Ctrl+C to end the call and exit".bright_yellow());
    println!("{}", "═══════════════════════════════════════════".bright_black());
}

async fn list_audio_devices(client: &SipClient) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n{}", "🎤 Audio Devices:".bright_white().bold());
    
    // List input devices
    let inputs = client.list_audio_devices(AudioDirection::Input).await?;
    println!("  {}:", "Microphones".bright_yellow());
    for (i, dev) in inputs.iter().enumerate() {
        println!("    {}. {}", i + 1, dev.name.bright_white());
    }
    
    // List output devices
    let outputs = client.list_audio_devices(AudioDirection::Output).await?;
    println!("  {}:", "Speakers".bright_yellow());
    for (i, dev) in outputs.iter().enumerate() {
        println!("    {}. {}", i + 1, dev.name.bright_white());
    }
    
    Ok(())
}

fn draw_audio_meter(level: f32) -> String {
    let bar_width = 20;
    let filled = (level * bar_width as f32) as usize;
    let empty = bar_width - filled;
    
    let color = if level > 0.8 {
        "red"
    } else if level > 0.5 {
        "yellow"
    } else {
        "green"
    };
    
    let bar = format!("{}{}", 
        "█".repeat(filled),
        "░".repeat(empty)
    );
    
    match color {
        "red" => bar.red().to_string(),
        "yellow" => bar.yellow().to_string(),
        _ => bar.green().to_string(),
    }
}