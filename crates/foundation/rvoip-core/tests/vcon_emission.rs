//! P3 acceptance — `VconReady` fires on `SessionEnded` and the
//! resulting handle resolves to a validated canonical vCon.

use bytes::Bytes;
use chrono::{Duration as ChronoDuration, Utc};
use rvoip_core::adapter::EndReason;
use rvoip_core::config::Config;
use rvoip_core::conversation::ConversationPolicy;
use rvoip_core::events::Event;
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{AttachmentId, ParticipantId, StreamId, TenantId};
use rvoip_core::orchestrator::Orchestrator;
use rvoip_core::participant::{ParticipantKind, ParticipantRole};
use rvoip_core::session::SessionMedium;
use rvoip_core::{
    VconAnalysis, VconAnalysisKind, VconAttachment, VconDialog, VconDialogKind, VconParty,
};
use std::collections::HashMap;
use std::time::Duration;
use uuid::Version;

async fn next_matching<F>(rx: &mut tokio::sync::broadcast::Receiver<Event>, mut f: F) -> Event
where
    F: FnMut(&Event) -> bool,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            panic!("timed out waiting for matching event");
        }
        let remaining = deadline - now;
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Ok(ev)) if f(&ev) => return ev,
            Ok(Ok(_)) => continue,
            Ok(Err(_)) => panic!("channel closed"),
            Err(_) => panic!("timed out"),
        }
    }
}

#[tokio::test]
async fn ending_a_session_emits_vcon_ready_with_resolvable_handle() {
    let orch = Orchestrator::new(Config::default());
    let cid = orch
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .unwrap();
    let sid = orch
        .start_session(cid.clone(), SessionMedium::Voice, vec![])
        .await
        .unwrap();
    // Two participants → vCon should snapshot both as parties.
    let first_party = ParticipantId::new();
    orch.join_session(
        sid.clone(),
        first_party.clone(),
        ParticipantKind::Human,
        ParticipantRole::Customer,
    )
    .await
    .unwrap();
    orch.join_session(
        sid.clone(),
        ParticipantId::new(),
        ParticipantKind::Ai,
        ParticipantRole::Agent,
    )
    .await
    .unwrap();

    let dialog_body = Bytes::from_static(b"quotes: \"hello\"\\world\ncontrol:\tend");
    let second_dialog_body = Bytes::from_static(b"second complete recording");
    let analysis_body = Bytes::from_static(b"{\"full\":\"analysis\\nbody\"}");
    let attachment_body = Bytes::from_static(b"\0full attachment bytes\xff");
    let stream_id = StreamId::new();
    let second_stream_id = StreamId::new();
    let started = Utc::now();
    let live = orch.session_vcon_handle(&sid).expect("live builder");
    let malicious_name = "Alice \"quoted\" \\\\ newline\nand\ttab";
    live.add_party(VconParty {
        participant_id: first_party.clone(),
        display_name: Some(malicious_name.into()),
        did: Some("did:example:alice".into()),
        stir: Some("header.payload.signature".into()),
        validation: IdentityAssurance::Anonymous,
    });
    live.add_dialog(VconDialog {
        kind: VconDialogKind::Audio,
        stream_id: Some(stream_id.clone()),
        started,
        ended: Some(started + ChronoDuration::milliseconds(1_250)),
        parties: vec![first_party.clone()],
        mediatype: Some("audio/opus".into()),
        body: Some(dialog_body.clone()),
        url: None,
        content_hash: None,
    });
    live.add_dialog(VconDialog {
        kind: VconDialogKind::Video,
        stream_id: Some(second_stream_id.clone()),
        started: started + ChronoDuration::milliseconds(250),
        ended: Some(started + ChronoDuration::milliseconds(1_500)),
        parties: vec![first_party.clone()],
        mediatype: Some("video/ogg".into()),
        body: Some(second_dialog_body.clone()),
        url: None,
        content_hash: None,
    });
    live.add_analysis(VconAnalysis {
        kind: VconAnalysisKind::Transcript,
        vendor: "rvoip-test".into(),
        product: Some("core-emission".into()),
        body: analysis_body.clone(),
        mediatype: "application/json".into(),
    });
    live.add_attachment(VconAttachment {
        id: AttachmentId::new(),
        started,
        party: first_party,
        dialog: second_stream_id,
        mediatype: "application/octet-stream".into(),
        body: attachment_body.clone(),
        purpose: Some("verbatim fixture".into()),
    });

    let mut events = orch.subscribe_events();

    orch.end_session(sid.clone(), EndReason::Normal)
        .await
        .unwrap();

    // VconReady arrives via a tokio::spawn so it may come after
    // SessionEnded on the broadcast bus.
    let ev = next_matching(&mut events, |e| matches!(e, Event::VconReady { .. })).await;
    let (sid_back, handle) = match ev {
        Event::VconReady {
            session_id, handle, ..
        } => (session_id, handle),
        _ => unreachable!(),
    };
    assert_eq!(sid_back, sid);
    assert!(handle.url.starts_with("memory:vcon/"));
    assert!(handle.content_hash.starts_with("sha512-"));

    // Verify the bytes resolve, hash-match, and are the actual canonical
    // document emitted by core (not a separately constructed fixture).
    let store = orch.config.vcon_store.clone();
    let bytes = store.get(&handle).await.unwrap().expect("bytes resolve");
    assert_eq!(handle.content_hash, rvoip_vcon::content_hash(&bytes));
    let conversation_handles = store.list_for_conversation(&cid).await.unwrap();
    assert_eq!(conversation_handles.len(), 1);
    assert_eq!(conversation_handles[0].url, handle.url);

    let document: rvoip_vcon::Vcon = serde_json::from_slice(&bytes).expect("canonical vCon JSON");
    document.validate().expect("valid vCon");
    assert_eq!(document.vcon.as_deref(), Some("0.4.0"));
    assert_eq!(document.uuid.get_version(), Some(Version::Custom));
    assert_eq!(document.parties.len(), 2);
    assert_eq!(document.parties[0].name.as_deref(), Some(malicious_name));
    assert_eq!(
        document.parties[0].did.as_deref(),
        Some("did:example:alice")
    );
    assert_eq!(
        document.parties[0].stir.as_deref(),
        Some("header.payload.signature")
    );
    assert_eq!(document.dialog.len(), 3);
    assert_eq!(document.dialog[0].duration, Some(1.25));
    assert_eq!(
        document.dialog[0].body,
        Some(serde_json::Value::String(rvoip_vcon::encode_base64url(
            &dialog_body
        )))
    );
    assert_eq!(
        document.dialog[1].body,
        Some(serde_json::Value::String(rvoip_vcon::encode_base64url(
            &second_dialog_body
        )))
    );
    assert_eq!(document.dialog[0].recording_set, Some(2));
    assert_eq!(document.dialog[1].recording_set, Some(2));
    assert_eq!(
        document.dialog[2].kind,
        rvoip_vcon::DialogKind::RecordingSet
    );
    assert_eq!(document.dialog[2].recordings, vec![0, 1]);
    assert_eq!(document.dialog[2].duration, Some(1.5));
    assert_eq!(document.attachments[0].dialog, 1);
    assert_eq!(
        document.analysis[0].body,
        Some(serde_json::Value::String(rvoip_vcon::encode_base64url(
            &analysis_body
        )))
    );
    assert_eq!(
        document.attachments[0].body,
        Some(serde_json::Value::String(rvoip_vcon::encode_base64url(
            &attachment_body
        )))
    );
}

#[tokio::test]
async fn session_vcon_handle_exposes_live_builder_during_session() {
    let orch = Orchestrator::new(Config::default());
    let pid = ParticipantId::new();
    let cid = orch
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .unwrap();
    let sid = orch
        .start_session(cid, SessionMedium::Voice, vec![pid.clone()])
        .await
        .unwrap();
    // Joining an invitee again must not duplicate the party in the
    // eventual canonical document.
    orch.join_session(
        sid.clone(),
        pid.clone(),
        ParticipantKind::Human,
        ParticipantRole::Customer,
    )
    .await
    .unwrap();

    let handle = orch.session_vcon_handle(&sid).expect("present");
    let snap = handle.snapshot();
    assert_eq!(snap.parties.len(), 1);
    assert_eq!(snap.parties[0].participant_id, pid);
    assert_eq!(snap.uuid.get_version(), Some(Version::Custom));
    assert!(snap.created_at <= Utc::now());
    let second_snapshot = handle.snapshot();
    assert_eq!(second_snapshot.uuid, snap.uuid);
    assert_eq!(second_snapshot.created_at, snap.created_at);
}

#[tokio::test]
async fn invalid_snapshot_is_not_stored_or_announced() {
    let orch = Orchestrator::new(Config::default());
    let cid = orch
        .open_conversation(
            TenantId::new(),
            ConversationPolicy::default(),
            HashMap::new(),
        )
        .await
        .unwrap();
    let sid = orch
        .start_session(cid, SessionMedium::Voice, vec![])
        .await
        .unwrap();

    let started = Utc::now();
    orch.session_vcon_handle(&sid)
        .expect("live builder")
        .add_dialog(VconDialog {
            kind: VconDialogKind::Text,
            stream_id: Some(StreamId::new()),
            started,
            ended: None,
            parties: vec![ParticipantId::new()],
            mediatype: Some("text/plain".into()),
            body: Some(Bytes::from_static(b"must not be stored")),
            url: None,
            content_hash: None,
        });

    let mut events = orch.subscribe_events();
    orch.end_session(sid.clone(), EndReason::Normal)
        .await
        .unwrap();

    // Session shutdown still completes.
    let ended = next_matching(&mut events, |event| {
        matches!(event, Event::SessionEnded { .. })
    })
    .await;
    assert!(matches!(ended, Event::SessionEnded { .. }));

    // Conversion failed synchronously, so no store task or VconReady event
    // can be produced.
    assert!(orch
        .config
        .vcon_store
        .list_for_session(&sid)
        .await
        .unwrap()
        .is_empty());
    assert!(
        tokio::time::timeout(Duration::from_millis(50), async {
            loop {
                if matches!(events.recv().await, Ok(Event::VconReady { .. })) {
                    return;
                }
            }
        })
        .await
        .is_err(),
        "VconReady was unexpectedly emitted"
    );
}
