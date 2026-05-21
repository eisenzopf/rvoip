//! Heap profile for the UDP receive path.
//!
//! Runs a fixed number of loopback packets through `UdpListener::receive`
//! (and, in a second pass, the full `UdpTransport` pipeline that emits
//! `TransportEvent::MessageReceived`) under dhat. Use this to spot
//! per-packet allocations in the transport layer.
//!
//! ```bash
//! cargo run --release --features dhat -p rvoip-sip --example profiling_dhat_udp
//! ```

#![cfg(feature = "dhat")]

use rvoip_sip_transport::transport::udp::UdpListener;
use rvoip_sip_transport::{Transport, TransportEvent, UdpTransport};
use std::hint::black_box;
use tokio::net::UdpSocket;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

const ITERATIONS: usize = 5_000;
const LOOPBACK: &str = "127.0.0.1:0";

const SAMPLE_INVITE: &[u8] = b"INVITE sip:bob@biloxi.example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP pc33.atlanta.example.com:5060;branch=z9hG4bKdhat\r\n\
Max-Forwards: 70\r\n\
To: Bob <sip:bob@biloxi.example.com>\r\n\
From: Alice <sip:alice@atlanta.example.com>;tag=dhat\r\n\
Call-ID: dhat-invite@pc33.atlanta.example.com\r\n\
CSeq: 1 INVITE\r\n\
Contact: <sip:alice@pc33.atlanta.example.com>\r\n\
Content-Length: 0\r\n\r\n";

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _profiler = dhat::Profiler::new_heap();

    // Pass 1: bare UdpListener::receive
    {
        let listener = UdpListener::bind(LOOPBACK.parse()?).await?;
        let listener_addr = listener.local_addr()?;
        let sender = UdpSocket::bind(LOOPBACK).await?;
        println!(
            "[dhat_udp] pass 1: {} UdpListener::receive cycles...",
            ITERATIONS
        );
        for _ in 0..ITERATIONS {
            sender.send_to(SAMPLE_INVITE, listener_addr).await?;
            let (bytes, _src, _local) = listener.receive().await?;
            black_box(bytes);
        }
    }

    // Pass 2: full UdpTransport pipeline (parse + event)
    {
        let (transport, mut events) =
            UdpTransport::bind(LOOPBACK.parse()?, Some(1024)).await?;
        let transport_addr = transport.local_addr()?;
        let sender = UdpSocket::bind(LOOPBACK).await?;
        println!(
            "[dhat_udp] pass 2: {} UdpTransport receive cycles...",
            ITERATIONS
        );
        for _ in 0..ITERATIONS {
            sender.send_to(SAMPLE_INVITE, transport_addr).await?;
            loop {
                match events.recv().await {
                    Some(TransportEvent::MessageReceived { message, .. }) => {
                        black_box(message);
                        break;
                    }
                    Some(_other) => continue,
                    None => panic!("transport channel closed"),
                }
            }
        }
        transport.close().await.ok();
    }

    println!("[dhat_udp] done — dhat-heap.json written");
    Ok(())
}
