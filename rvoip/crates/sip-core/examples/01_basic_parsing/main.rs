//! Basic SIP Message Parsing Examples
//! 
//! This program provides a menu to run different SIP parsing examples.
//! Each example demonstrates a specific aspect of working with SIP messages.

use std::io::{self, Write};
use std::process::Command;

fn main() {
    println!("SIP Core Basic Parsing Examples");
    println!("===============================\n");
    
    loop {
        println!("\nSelect an example to run:");
        println!("1. Parsing a SIP INVITE request (using typed headers)");
        println!("1a. Parsing a SIP INVITE request (using path accessors)");
        println!("1b. Parsing a SIP INVITE request (using query accessors)");
        println!("2. Parsing a SIP response");
        println!("3. Parsing a message with multiple headers");
        println!("4. Creating SIP messages with SDP content");
        println!("5. Run all examples");
        println!("0. Exit");
        
        print!("\nEnter your choice: ");
        io::stdout().flush().unwrap();
        
        let mut choice = String::new();
        io::stdin().read_line(&mut choice).expect("Failed to read input");
        
        match choice.trim() {
            "1" => run_example("01_invite_request_typed"),
            "1a" => run_example("01_invite_request_path"),
            "1b" => run_example("01_invite_request_query"),
            "2" => run_example("02_sip_response"),
            "3" => run_example("03_multiple_headers"),
            "4" => run_example("04_sdp_builder"),
            "5" => {
                println!("\nRunning all examples sequentially...\n");
                run_example("01_invite_request_typed");
                run_example("01_invite_request_path");
                run_example("01_invite_request_query");
                run_example("02_sip_response");
                run_example("03_multiple_headers");
                run_example("04_sdp_builder");
            },
            "0" => {
                println!("Exiting...");
                break;
            },
            _ => println!("Invalid choice. Please try again."),
        }
    }
}

fn run_example(name: &str) {
    println!("\n----- Running example: {} -----\n", name);
    
    // Using cargo run to run each individual example
    // Set RUST_LOG=info to ensure tracing messages are displayed
    let status = Command::new("cargo")
        .args(["run", "--example", name])
        .env("RUST_LOG", "info")
        .status()
        .expect("Failed to execute cargo run command");
    
    if !status.success() {
        println!("\nExample {} failed with exit code: {:?}", name, status.code());
    }
    
    println!("\n----- End of example: {} -----", name);
} 