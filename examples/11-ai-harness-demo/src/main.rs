use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use rvoip_core_traits::ids::{ConnectionId, ParticipantId, StreamId};
use rvoip_core_traits::stream::{MediaFrame, StreamKind};
use rvoip_harness::{
    AsrConfig, AsrProvider, AsrResult, AsrStream, DialogAction, DialogManager, RecordingSink,
    TtsPlayback, TtsProvider, TtsRequest, VecRecordingSink,
};
use rvoip_vcon::{DialogKind, Party, VconBuilder};

struct FakeAsrProvider;

struct FakeAsrStream {
    stream_id: StreamId,
    speaker: ParticipantId,
    queued: Mutex<VecDeque<AsrResult>>,
}

#[async_trait]
impl AsrProvider for FakeAsrProvider {
    async fn open_stream(
        &self,
        _conn: ConnectionId,
        _config: AsrConfig,
    ) -> rvoip_core_traits::error::Result<Box<dyn AsrStream>> {
        Ok(Box::new(FakeAsrStream {
            stream_id: StreamId::new(),
            speaker: ParticipantId::from_string("part_caller"),
            queued: Mutex::new(VecDeque::new()),
        }))
    }
}

#[async_trait]
impl AsrStream for FakeAsrStream {
    async fn push(&self, frame: MediaFrame) -> rvoip_core_traits::error::Result<()> {
        let text = format!("caller said {} bytes", frame.payload.len());
        self.queued
            .lock()
            .expect("fake ASR queue lock poisoned")
            .push_back(AsrResult {
                stream_id: self.stream_id.clone(),
                speaker: Some(self.speaker.clone()),
                text,
                confidence: 1.0,
                is_final: true,
            });
        Ok(())
    }

    async fn next(&self) -> Option<AsrResult> {
        self.queued
            .lock()
            .expect("fake ASR queue lock poisoned")
            .pop_front()
    }

    async fn close(&self) -> rvoip_core_traits::error::Result<()> {
        Ok(())
    }
}

struct FakeDialog;

#[async_trait]
impl DialogManager for FakeDialog {
    async fn turn(&self, transcript: &AsrResult) -> rvoip_core_traits::error::Result<DialogAction> {
        Ok(DialogAction::Say {
            text: format!("ack: {}", transcript.text),
            voice: Some("test-voice".into()),
        })
    }
}

struct FakeTtsProvider;

struct FakeTtsPlayback {
    next: Mutex<Option<MediaFrame>>,
}

#[async_trait]
impl TtsProvider for FakeTtsProvider {
    async fn synthesize(
        &self,
        request: TtsRequest,
    ) -> rvoip_core_traits::error::Result<Box<dyn TtsPlayback>> {
        Ok(Box::new(FakeTtsPlayback {
            next: Mutex::new(Some(MediaFrame {
                stream_id: StreamId::new(),
                kind: StreamKind::Audio,
                payload: Bytes::from(request.text),
                timestamp_rtp: 0,
                captured_at: Utc::now(),
                payload_type: None,
            })),
        }))
    }
}

#[async_trait]
impl TtsPlayback for FakeTtsPlayback {
    async fn next_frame(&self) -> Option<MediaFrame> {
        self.next
            .lock()
            .expect("fake TTS playback lock poisoned")
            .take()
    }

    async fn cancel(&self) -> rvoip_core_traits::error::Result<()> {
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let conn = ConnectionId::new();
    let input = MediaFrame {
        stream_id: StreamId::new(),
        kind: StreamKind::Audio,
        payload: Bytes::from_static(b"deterministic caller audio"),
        timestamp_rtp: 0,
        captured_at: Utc::now(),
        payload_type: None,
    };

    let asr = FakeAsrProvider;
    let asr_stream = asr.open_stream(conn, AsrConfig::default()).await?;
    asr_stream.push(input).await?;
    let transcript = asr_stream.next().await.expect("fake transcript");

    let dialog = FakeDialog;
    let response = match dialog.turn(&transcript).await? {
        DialogAction::Say { text, voice } => {
            let tts = FakeTtsProvider;
            let playback = tts
                .synthesize(TtsRequest {
                    voice,
                    text: text.clone(),
                    sample_rate_hz: Some(8_000),
                })
                .await?;
            let sink = VecRecordingSink::new("memory://ai-harness-demo.raw");
            while let Some(frame) = playback.next_frame().await {
                sink.write(frame).await?;
            }
            let artifact = sink.close().await?;
            (text, artifact)
        }
        DialogAction::Listen => (
            "listen".into(),
            VecRecordingSink::new("memory://empty").close().await?,
        ),
        DialogAction::End => (
            "end".into(),
            VecRecordingSink::new("memory://empty").close().await?,
        ),
    };

    let mut builder = VconBuilder::new().subject("AI harness demo");
    let caller = builder.party(Party {
        name: Some("Caller".into()),
        role: Some("caller".into()),
        uuid: transcript.speaker.as_ref().map(ToString::to_string),
        ..Party::default()
    });
    let bot = builder.party(Party {
        name: Some("Deterministic Bot".into()),
        role: Some("bot".into()),
        uuid: Some("part_bot".into()),
        ..Party::default()
    });
    let vcon = builder
        .text(Utc::now(), caller, transcript.text.clone())
        .text(Utc::now(), bot, response.0.clone())
        .recording(
            Utc::now(),
            response.1.duration_ms,
            vec![caller, bot],
            "audio/raw",
        )
        .build();

    println!("transcript: {}", transcript.text);
    println!("response: {}", response.0);
    println!(
        "recording: {} bytes at {}",
        response.1.bytes_written, response.1.url
    );
    println!("vcon: {} dialogs={}", vcon.uuid, vcon.dialog.len());
    println!(
        "evidence: transcript + {:?} dialog + recording artifact",
        DialogKind::Recording
    );

    Ok(())
}
