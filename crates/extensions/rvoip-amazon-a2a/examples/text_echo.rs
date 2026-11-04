//! Deterministic text-mode Connect smoke server. Put a TLS reverse proxy in
//! front of 127.0.0.1:8080; never expose the plaintext socket publicly.
use rvoip_amazon_a2a::{
    AmazonA2aAdapter, AmazonA2aConfig, ApiKey, CallBinding, FinishType, InputEvent, TurnTrace,
};
use rvoip_core::{
    adapter::ConnectionAdapter,
    ids::{ParticipantId, SessionId},
};
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

fn now() -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_nanos()
        .try_into()?)
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let adapter = AmazonA2aAdapter::new(AmazonA2aConfig::new(ApiKey::new(std::env::var(
        "AMAZON_A2A_API_KEY",
    )?)?))?;
    let instance = std::env::var("CONNECT_INSTANCE_ARN")?;
    let mut events = adapter.subscribe_events();
    tokio::spawn(async move { while events.recv().await.is_some() {} });
    let port: u16 = std::env::var("AMAZON_A2A_PORT")
        .unwrap_or_else(|_| "8080".into())
        .parse()?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    eprintln!("A2A text echo listening on loopback port {port}; production requires TLS ingress");
    loop {
        tokio::select! {
            _=tokio::signal::ctrl_c()=>break,
            accepted=listener.accept()=>{
                let (stream,_)=accepted?;let adapter=Arc::clone(&adapter);let instance=instance.clone();
                eprintln!("A2A smoke transport accepted");
                tokio::spawn(async move {
                    let run=async {
                        let mut call=adapter.accept_websocket(stream,CallBinding {session_id:SessionId::new(),participant_id:ParticipantId::new(),instance_arn:instance}).await?;
                        eprintln!("A2A smoke session initialized");
                        let result=async {
                            if call.initialization().is_audio(){return Err("this example supports text collaboration only".into());}
                            // Direct acceptance is for this isolated single-tenant smoke
                            // server. Orchestrated applications use route_inbound_connection.
                            adapter.accept(call.connection_id().clone()).await?;
                            while let Some(input)=call.next_input().await {
                                if matches!(input,InputEvent::Ready){eprintln!("A2A smoke channel ready");}
                                if let InputEvent::Text(text)=input {
                                    let start=now()?;let response=format!("You said: {text}");
                                    call.send_text(response.clone()).await?;
                                    call.complete_turn(TurnTrace::new("text-echo",text.clone(),response,start,now()?)?).await?;
                                    eprintln!("A2A smoke text turn completed");
                                    if text.eq_ignore_ascii_case("goodbye") {
                                        call.finish(FinishType::Complete,"Echo smoke test complete").await?;
                                        eprintln!("A2A smoke collaboration finished");break;
                                    }
                                }
                            }
                            Ok::<(),Box<dyn std::error::Error+Send+Sync>>(())
                        }.await;
                        call.close().await;
                        result
                    }.await;
                    // Do not print raw peer errors or conversation content.
                    if run.is_err(){eprintln!("A2A smoke session failed");}
                });
            }
        }
    }
    adapter.shutdown().await;
    Ok(())
}
