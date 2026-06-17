# 11-ai-harness-demo

In-process AI voice harness demo using deterministic fake providers. It does not call external ASR, TTS, dialog, or storage services.

Run:

```bash
cargo run
```

The demo pushes one synthetic audio frame through fake ASR, turns the transcript into a deterministic dialog response, writes fake TTS audio into `VecRecordingSink`, and builds vCon evidence containing the transcript and bot response.
