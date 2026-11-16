use axum::{
    extract::{
        ws::{Message, WebSocketUpgrade},
        State,
    },
    routing::{get, post},
    Json, Router,
};
use rvoip_vapi::{
    raw_session::{RawVapiItem, RawVapiSession},
    VapiApiKey, VapiAssistant, VapiCallOptions, VapiConfig, VapiEvent,
};
use serde_json::{json, Value};
use std::time::Duration;

#[tokio::test]
async fn preserves_wire_order_and_ends_call_with_correct_command() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (observed, mut received) = tokio::sync::mpsc::channel::<Value>(4);
    let app=Router::new().route("/call",post(move|Json(body):Json<Value>|async move {
        assert_eq!(body["transport"]["audioFormat"]["format"],"mulaw");
        Json(json!({"id":"raw-test","transport":{"websocketCallUrl":format!("ws://{addr}/ws")}}))
    })).route("/ws",get(|State(observed):State<tokio::sync::mpsc::Sender<Value>>,ws:WebSocketUpgrade|async move {
        ws.on_upgrade(move|mut socket|async move {
            socket.send(Message::Binary(vec![1;8000])).await.unwrap();
            socket.send(Message::Text(json!({"type":"speech-update","role":"assistant","status":"stopped"}).to_string())).await.unwrap();
            socket.send(Message::Binary(vec![2;160])).await.unwrap();
            while let Some(Ok(message))=socket.recv().await {
                if let Message::Text(text)=message {
                    let value:Value=serde_json::from_str(&text).unwrap();
                    observed.send(value).await.unwrap();
                }
            }
        })
    })).with_state(observed);
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut config = VapiConfig::new(VapiApiKey::new("test-only-key").unwrap());
    config.api_base = format!("http://{addr}/").parse().unwrap();
    config = config.with_loopback_test_transport();
    let mut session = RawVapiSession::open(
        config,
        VapiCallOptions::new(VapiAssistant::saved("assistant-test")),
    )
    .await
    .unwrap();
    // One full second of audio must be delivered without waiting for a media clock.
    tokio::time::timeout(Duration::from_millis(500),async {
        assert!(matches!(session.next().await.unwrap(),RawVapiItem::Audio(bytes) if bytes.len()==8000));
        assert!(matches!(session.next().await.unwrap(),RawVapiItem::Event(VapiEvent::SpeechUpdate{status,..}) if status=="stopped"));
        assert!(matches!(session.next().await.unwrap(),RawVapiItem::Audio(bytes) if bytes.as_ref()==[2;160]));
    }).await.unwrap();
    session.close().await.unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), received.recv())
            .await
            .unwrap()
            .unwrap(),
        json!({"type":"end-call"})
    );
    server.abort();
}
