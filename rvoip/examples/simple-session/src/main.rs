use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::info;
use uuid::Uuid;

use rvoip_session_core::{
    SessionState, SessionEvent, Error as SessionError, 
    SessionId,
};

/// Command line arguments
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

/// Subcommands
#[derive(Subcommand, Debug)]
enum Command {
    /// Print information about session-core
    Info,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();
    
    // Parse command line arguments
    let args = Args::parse();
    
    // Run the appropriate mode
    match args.command {
        Command::Info => {
            print_info().await?;
        }
    }
    
    Ok(())
}

/// Print information about session-core
async fn print_info() -> Result<()> {
    info!("Session Core Information");
    info!("=======================");
    
    // Print session states
    info!("Session States:");
    info!("  {:?}", SessionState::Initializing);
    info!("  {:?}", SessionState::Dialing);
    info!("  {:?}", SessionState::Ringing);
    info!("  {:?}", SessionState::Connected);
    info!("  {:?}", SessionState::Terminating);
    info!("  {:?}", SessionState::Terminated);
    
    // Create a sample session ID
    let session_id = SessionId(Uuid::new_v4());
    
    // Print session events
    info!("Session Events Examples:");
    info!("  {:?}", SessionEvent::Created { 
        session_id: session_id.clone(),
    });
    info!("  {:?}", SessionEvent::StateChanged { 
        session_id: session_id.clone(),
        old_state: SessionState::Ringing,
        new_state: SessionState::Connected,
    });
    info!("  {:?}", SessionEvent::Terminated { 
        session_id,
        reason: "Call completed normally".to_string(),
    });
    
    // Print session error examples
    info!("Session Error Examples:");
    info!("  {:?}", SessionError::SessionNotFound("12345".to_string()));
    info!("  {:?}", SessionError::DialogNotFound("12345".to_string()));
    info!("  {:?}", SessionError::InvalidStateTransition(
        "idle".to_string(),
        "terminated".to_string(),
    ));
    
    Ok(())
} 