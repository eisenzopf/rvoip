use base64::{engine::general_purpose::STANDARD, Engine};
use rvoip_webex_byob::{Input, ProtocolError, Session};
use serde_json::{json, Value};

fn request(seq: u64, input: Value) -> String {
    json!({"type":"VOICE_VA_REQUEST","seq":seq,"ts":"2026-10-28T12:00:00Z",
        "conversation_id":"conversation","payload":{"conversation_id":"conversation",
        "customer_org_id":"tenant","virtual_agent_id":"agent","voice_va_input_type":input}})
    .to_string()
}

fn started() -> Session {
    let mut session = Session::new("tenant".into(), "agent".into());
    assert!(matches!(
        session.receive(&request(
            1,
            json!({"event_input":{"event_type":"SESSION_START"}})
        )),
        Ok(Input::Start)
    ));
    session
}

#[test]
fn cisco_null_optional_maps_preserve_required_field_validation() {
    let mut message: Value = serde_json::from_str(&request(
        1,
        json!({"event_input":{"event_type":"SESSION_START","name":null,"parameters":null}}),
    ))
    .unwrap();
    message["metadata"] = Value::Null;
    message["payload"]["additional_info"] = Value::Null;
    let mut session = Session::new("tenant".into(), "agent".into());
    assert!(matches!(
        session.receive(&message.to_string()),
        Ok(Input::Start)
    ));
    let mut audio: Value = serde_json::from_str(&request(2, json!({"audio_input":{
        "caller_audio_b64":STANDARD.encode([0x7f;640]),"encoding":"MULAW_FORMAT","sample_rate_hertz":8000
    }}))).unwrap();
    audio["metadata"] = Value::Null;
    audio["payload"]["additional_info"] = Value::Null;
    assert!(
        matches!(session.receive(&audio.to_string()), Ok(Input::Audio(bytes)) if bytes.len()==640)
    );
    for (pointer, value) in [
        ("/metadata", json!([])),
        ("/payload/additional_info", json!({"key":null})),
        ("/payload/additional_info", json!("invalid")),
        ("/payload/customer_org_id", Value::Null),
        ("/conversation_id", Value::Null),
        ("/seq", json!(0)),
    ] {
        let mut bad = message.clone();
        *bad.pointer_mut(pointer).unwrap() = value;
        assert!(
            Session::new("tenant".into(), "agent".into())
                .receive(&bad.to_string())
                .is_err(),
            "{pointer}"
        );
    }
    let mut wrong_org = message;
    wrong_org["payload"]["customer_org_id"] = json!("other-tenant");
    assert_eq!(
        Session::new("tenant".into(), "agent".into())
            .receive(&wrong_org.to_string())
            .unwrap_err(),
        ProtocolError::Identity
    );
}

fn decoded(messages: &[Value]) -> Vec<u8> {
    let mut audio = Vec::new();
    for message in messages {
        if let Some(prompts) = message["payload"]["prompts"].as_array() {
            for prompt in prompts {
                let chunk = STANDARD
                    .decode(prompt["audio_content_b64"].as_str().unwrap())
                    .unwrap();
                assert!((320..=32000).contains(&chunk.len()));
                audio.extend(chunk);
            }
        }
    }
    audio
}

#[test]
fn arbitrary_audio_partitions_preserve_bytes_and_final_order() {
    let audio: Vec<u8> = (0..75003).map(|n| (n % 251) as u8).collect();
    for split in [1, 17, 159, 160, 319, 320, 321, 640, 8192, 32000, 75003] {
        let mut session = started();
        let id = session.begin_reply().unwrap();
        let mut output = Vec::new();
        for chunk in audio.chunks(split) {
            output.extend(session.audio(id, chunk).unwrap());
        }
        output.extend(session.finish(id).unwrap());
        assert_eq!(decoded(&output), audio);
        assert_eq!(output.last().unwrap()["payload"]["response_type"], "FINAL");
        assert_eq!(
            output
                .iter()
                .filter(|m| m["payload"]["response_type"] == "FINAL")
                .count(),
            1
        );
        assert!(output
            .windows(2)
            .all(|p| p[0]["seq"].as_u64() < p[1]["seq"].as_u64()));
        assert_eq!(session.finish(id).unwrap_err(), ProtocolError::StaleReply);
        assert_eq!(
            session.audio(id, &[1; 640]).unwrap_err(),
            ProtocolError::StaleReply
        );
    }
}

#[test]
fn cancellation_fences_old_generation_and_discards_tail() {
    let mut session = started();
    let old = session.begin_reply().unwrap();
    assert!(session.audio(old, &[99; 319]).unwrap().is_empty());
    let interrupt = session.interrupt().unwrap();
    assert_eq!(
        interrupt[0]["payload"]["output_events"][0]["event_type"],
        "START_OF_INPUT"
    );
    assert_eq!(interrupt[1]["payload"]["response_type"], "FINAL");
    assert!(decoded(&interrupt).is_empty());
    let new = session.begin_reply().unwrap();
    assert_ne!(old, new);
    assert_eq!(
        session.audio(old, &[1; 640]).unwrap_err(),
        ProtocolError::StaleReply
    );
    assert_eq!(session.interrupt().unwrap().len(), 2);
    assert_eq!(session.interrupt().unwrap().len(), 1);
}

#[test]
fn short_utterance_is_explicit_error_and_never_silently_dropped() {
    let mut session = started();
    let id = session.begin_reply().unwrap();
    session.audio(id, &[1; 319]).unwrap();
    assert_eq!(session.finish(id).unwrap_err(), ProtocolError::ShortAudio);
    session.audio(id, &[1]).unwrap();
    assert_eq!(decoded(&session.finish(id).unwrap()).len(), 320);
}

#[test]
fn first_message_identity_and_sequences_are_enforced() {
    let mut session = Session::new("tenant".into(), "agent".into());
    assert_eq!(
        session
            .receive(&request(
                1,
                json!({"event_input":{"event_type":"NO_INPUT"}})
            ))
            .unwrap_err(),
        ProtocolError::State
    );
    let mut session = started();
    assert_eq!(
        session
            .receive(&request(
                1,
                json!({"event_input":{"event_type":"NO_INPUT"}})
            ))
            .unwrap_err(),
        ProtocolError::Sequence
    );
    assert!(matches!(
        session.receive(&request(
            7,
            json!({"event_input":{"event_type":"NO_INPUT"}})
        )),
        Ok(Input::NoInput)
    ));
    let wrong =
        request(8, json!({"event_input":{"event_type":"NO_INPUT"}})).replace("tenant", "other");
    assert_eq!(
        session.receive(&wrong).unwrap_err(),
        ProtocolError::Identity
    );
    let mismatch = request(8, json!({"event_input":{"event_type":"NO_INPUT"}})).replacen(
        "conversation",
        "different",
        1,
    );
    assert!(session.receive(&mismatch).is_err());
}

#[test]
fn heartbeats_echo_without_reusing_application_sequence() {
    let mut session = started();
    let ping = json!({"type":"PING","seq":50,"ts":"2026-10-28T12:00:00Z","conversation_id":"conversation"});
    assert!(matches!(
        session.receive(&ping.to_string()),
        Ok(Input::Ping(50))
    ));
    assert_eq!(session.pong(50).unwrap()["seq"], 50);
    let id = session.begin_reply().unwrap();
    assert_eq!(session.finish(id).unwrap()[0]["seq"], 1);
}

#[test]
fn dtmf_preserves_order_and_validates_digits() {
    let mut session = started();
    let input = json!({"dtmf_input":{"dtmf_events":["DTMF_DIGIT_ONE","DTMF_DIGIT_A","DTMF_DIGIT_STAR","DTMF_DIGIT_POUND","DTMF_DIGIT_ZERO"]}});
    assert!(matches!(session.receive(&request(2,input)),Ok(Input::Dtmf(d)) if d=="1A*#0"));
    assert!(session
        .receive(&request(3, json!({"dtmf_input":{"dtmf_events":["1"]}})))
        .is_err());
}

#[test]
fn malformed_media_is_rejected_without_content_in_errors() {
    for (encoding, rate, data) in [
        ("LINEAR16_FORMAT", 8000, STANDARD.encode([0; 320])),
        ("MULAW_FORMAT", 16000, STANDARD.encode([0; 320])),
        ("MULAW_FORMAT", 8000, "secret-invalid-base64".into()),
        ("MULAW_FORMAT", 8000, STANDARD.encode(b"RIFFsecret")),
    ] {
        let mut session = started();
        let err=session.receive(&request(2,json!({"audio_input":{"encoding":encoding,"sample_rate_hertz":rate,"caller_audio_b64":data}}))).unwrap_err();
        assert!(!format!("{err:?}").contains("secret"));
    }
}

#[test]
fn wrappers_unknown_fields_and_duplicate_fields_fail() {
    let mut session = started();
    assert!(session.receive(&request(2,json!({"event_input":{"event_type":"NO_INPUT"},"dtmf_input":{"dtmf_events":["DTMF_DIGIT_ONE"]}}))).is_err());
    assert!(session
        .receive(&request(
            2,
            json!({"event_input":{"event_type":"BARGE_IN"}})
        ))
        .is_err());
    let duplicate = request(2, json!({"event_input":{"event_type":"NO_INPUT"}})).replacen(
        "{",
        "{\"seq\":3,",
        1,
    );
    assert!(session.receive(&duplicate).is_err());
    let unknown = request(
        2,
        json!({"event_input":{"event_type":"NO_INPUT","surprise":true}}),
    );
    assert!(session.receive(&unknown).is_err());
}

#[test]
fn teardown_is_terminal_and_multiple_replies_share_connection() {
    let mut session = started();
    for _ in 0..3 {
        let id = session.begin_reply().unwrap();
        assert_eq!(session.finish(id).unwrap().len(), 1);
    }
    assert!(matches!(
        session.receive(&request(
            2,
            json!({"event_input":{"event_type":"SESSION_END"}})
        )),
        Ok(Input::End)
    ));
    assert!(session.begin_reply().is_err());
    assert!(session
        .receive(&request(
            3,
            json!({"event_input":{"event_type":"SESSION_START"}})
        ))
        .is_err());
}

#[cfg(feature = "wav-compat")]
#[test]
fn wav_compat_bounds_each_container_preserves_samples_and_finality() {
    use rvoip_webex_byob::AudioOutput;
    let audio: Vec<u8> = (0..75003).map(|n| (n % 256) as u8).collect();
    let expected = rvoip_webex_byob::wav::pcmu_to_pcm16_wav(&audio).unwrap();
    for split in [1, 319, 640, 8192, 32000, 75003] {
        let mut session = started().with_audio_output(AudioOutput::Pcm16Wav);
        let id = session.begin_reply().unwrap();
        let mut messages = Vec::new();
        for part in audio.chunks(split) {
            messages.extend(session.audio(id, part).unwrap());
        }
        messages.extend(session.finish(id).unwrap());
        let mut pcm = Vec::new();
        assert_eq!(
            messages
                .iter()
                .filter(|m| m["payload"]["response_type"] == "FINAL")
                .count(),
            1
        );
        for message in &messages {
            assert_eq!(message["payload"]["disable_prompt_cancellation"], true);
            for prompt in message["payload"]["prompts"].as_array().unwrap() {
                let wav = STANDARD
                    .decode(prompt["audio_content_b64"].as_str().unwrap())
                    .unwrap();
                assert!(wav.len() <= 32000);
                assert_eq!(&wav[..4], b"RIFF");
                assert_eq!(&wav[8..16], b"WAVEfmt ");
                assert_eq!(&wav[20..24], &[1, 0, 1, 0]);
                assert_eq!(&wav[24..28], &8000u32.to_le_bytes());
                assert_eq!(
                    u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize,
                    wav.len() - 44
                );
                pcm.extend_from_slice(&wav[44..]);
            }
        }
        assert_eq!(pcm, &expected[44..]);
        assert!(session.audio(id, &[0xff; 640]).is_err());
    }
}

#[cfg(feature = "wav-compat")]
#[test]
fn wav_aggregates_small_frames_and_discards_interrupted_tail() {
    use rvoip_webex_byob::AudioOutput;
    let mut session = started().with_audio_output(AudioOutput::Pcm16Wav);
    let id = session.begin_reply().unwrap();
    let mut messages = Vec::new();
    for chunk in vec![42; 80_000].chunks(341) {
        messages.extend(session.audio(id, chunk).unwrap());
    }
    messages.extend(session.finish(id).unwrap());
    assert_eq!(messages.len(), 10);
    for message in &messages {
        let wav = STANDARD
            .decode(
                message["payload"]["prompts"][0]["audio_content_b64"]
                    .as_str()
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(wav.len(), 16_044);
    }
    assert_eq!(
        messages.last().unwrap()["payload"]["response_type"],
        "FINAL"
    );
    let old = session.begin_reply().unwrap();
    assert!(session.audio(old, &[42; 7000]).unwrap().is_empty());
    session.interrupt().unwrap();
    assert!(session.audio(old, &[42; 341]).is_err());
    let next = session.begin_reply().unwrap();
    assert!(session.audio(next, &[0xff; 400]).unwrap().is_empty());
    let tail = session.finish(next).unwrap();
    let wav = STANDARD
        .decode(
            tail[0]["payload"]["prompts"][0]["audio_content_b64"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(wav.len(), 844);
    assert!(wav[44..].iter().all(|&b| b == 0));
}

#[cfg(feature = "wav-compat")]
#[test]
fn whole_wav_is_one_final_preserves_audio_and_bounds_buffer() {
    use rvoip_webex_byob::{wav, AudioOutput};
    let audio: Vec<u8> = (0..160_003).map(|n| (n % 256) as u8).collect();
    let mut session = started().with_audio_output(AudioOutput::Pcm16WavWhole);
    let id = session.begin_reply().unwrap();
    for part in audio.chunks(341) {
        assert!(session.audio(id, part).unwrap().is_empty());
    }
    let messages = session.finish(id).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["payload"]["response_type"], "FINAL");
    let actual = STANDARD
        .decode(
            messages[0]["payload"]["prompts"][0]["audio_content_b64"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(actual, wav::pcmu_to_pcm16_wav(&audio).unwrap());
    assert!(actual.len() > 32_000);
    assert!(session.finish(id).is_err());
    let id = session.begin_reply().unwrap();
    for _ in 0..2 {
        assert!(session
            .audio(id, &vec![42; wav::MAX_WAV_SAMPLES / 2])
            .unwrap()
            .is_empty());
    }
    assert_eq!(session.audio(id, &[42]).unwrap_err(), ProtocolError::Limit);
    session.interrupt().unwrap();
    assert!(session.finish(id).is_err());
    let id = session.begin_reply().unwrap();
    assert!(session.audio(id, &[0xff; 400]).unwrap().is_empty());
    let messages = session
        .complete_with_metadata("context_result", json!({"resultCode":"Friday-2-PM"}))
        .unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["payload"]["response_type"], "FINAL");
    let audio = STANDARD
        .decode(
            messages[0]["payload"]["prompts"][0]["audio_content_b64"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(audio.len(), 844);
    assert!(audio[44..].iter().all(|&b| b == 0));
    assert!(session.is_ended());
}

#[test]
fn startup_context_and_result_metadata_are_bounded_and_call_scoped() {
    let mut s = Session::new("tenant".into(), "agent".into());
    let context = json!({"context":{"customerName":"Alex","caseId":"APT-2048","verificationWord":"Thursday-10-AM"}});
    s.receive(&request(
        1,
        json!({"event_input":{"event_type":"SESSION_START","parameters":context}}),
    ))
    .unwrap();
    assert_eq!(s.start_parameters(), &context);
    assert!(s
        .receive(&request(
            2,
            json!({"event_input":{"event_type":"SESSION_START","parameters":{}}})
        ))
        .is_err());
    assert_eq!(s.start_parameters(), &context);
    let reply = s.begin_reply().unwrap();
    let _ = s.audio(reply, &[0xff; 640]).unwrap();
    let result = json!({"caseId":"APT-2048","resultCode":"Friday-2-PM"});
    let messages = s
        .complete_with_metadata("context_result", result.clone())
        .unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["payload"]["response_type"], "FINAL");
    assert_eq!(
        messages[0]["payload"]["output_events"][0]["metadata"],
        result
    );
    assert_eq!(
        messages[0]["payload"]["output_events"][1]["event_type"],
        "SESSION_END"
    );
    assert!(s.is_ended());
    assert!(s
        .complete_with_metadata("context_result", json!({}))
        .is_err());
    let mut s = started();
    assert!(s
        .complete_with_metadata("context_result", json!({"x":"a".repeat(16385)}))
        .is_err());
    assert!(!s.is_ended());
    let mut s = Session::new("tenant".into(), "agent".into());
    assert!(s.receive(&request(1, json!({"event_input":{"event_type":"SESSION_START","parameters":{"x":"a".repeat(16385)}}}))).is_err());
    assert!(s.conversation_id().is_none());
}

#[cfg(feature = "wav-compat")]
#[test]
fn fast_answers_preserve_intro_and_samples_without_tiny_prompt_train() {
    use rvoip_webex_byob::AudioOutput;
    let mut session = started()
        .with_audio_output(AudioOutput::Pcm16Wav)
        .with_answer_audio_output(AudioOutput::Pcm16WavFastStart);
    let intro = session.begin_reply().unwrap();
    assert!(session.audio(intro, &[42; 682]).unwrap().is_empty());
    session.finish(intro).unwrap();
    let audio: Vec<u8> = (0..80_000).map(|n| (n % 256) as u8).collect();
    for _ in 0..2 {
        let reply = session.begin_reply().unwrap();
        assert!(session.audio(reply, &audio[..341]).unwrap().is_empty());
        let mut messages = session.audio(reply, &audio[341..682]).unwrap();
        assert_eq!(messages.len(), 1); // First prompt after two provider frames, not one second.
        let first = STANDARD
            .decode(
                messages[0]["payload"]["prompts"][0]["audio_content_b64"]
                    .as_str()
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(first.len(), 44 + 362 * 2);
        for frame in audio[682..].chunks(341) {
            messages.extend(session.audio(reply, frame).unwrap());
        }
        messages.extend(session.finish(reply).unwrap());
        assert_eq!(messages.len(), 11); // Only one extra prompt, not hundreds of tiny WAVs.
        assert_eq!(
            messages
                .iter()
                .filter(|m| m["payload"]["response_type"] == "FINAL")
                .count(),
            1
        );
        let actual: Vec<u8> = messages
            .iter()
            .flat_map(|m| {
                let wav = STANDARD
                    .decode(
                        m["payload"]["prompts"][0]["audio_content_b64"]
                            .as_str()
                            .unwrap(),
                    )
                    .unwrap();
                wav[44..].to_vec()
            })
            .collect();
        assert_eq!(
            actual,
            rvoip_webex_byob::wav::pcmu_to_pcm16_wav(&audio).unwrap()[44..]
        );
    }
    let old = session.begin_reply().unwrap();
    session.audio(old, &[42; 682]).unwrap();
    session.interrupt().unwrap();
    assert!(session.audio(old, &[42; 682]).is_err());
    let next = session.begin_reply().unwrap();
    assert_eq!(session.audio(next, &[42; 682]).unwrap().len(), 1);
}
