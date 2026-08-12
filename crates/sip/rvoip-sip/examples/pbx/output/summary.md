# PBX Interop Run Summary

- started_at_utc: 2026-08-12T02:05:51Z
- ended_at_utc: 2026-08-12T02:06:04Z
- duration_seconds: 13
- exit_status: 0
- output_root: /Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output
- environments: `environment-*.md`
- matrix: `matrix.tsv`

## Result

- total_cells: 4
- passed_cells: 4
- failed_cells: 0

## Matrix

| Status | Provider | API | Scenario | Codec | Transport | Role | Duration | Exit | Log |
|--------|----------|-----|----------|-------|-----------|------|----------|------|-----|
| PASS | freeswitch | endpoint | amr_transcode_call | amrnb_be_pcmu | TLS | caller | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/freeswitch/endpoint/amr_transcode_call/amrnb_be_pcmu/TLS/caller.log` |
| PASS | freeswitch | endpoint | amr_transcode_call | amrnb_be_pcmu | TLS | callee | 5s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/freeswitch/endpoint/amr_transcode_call/amrnb_be_pcmu/TLS/callee.log` |
| PASS | freeswitch | endpoint | amr_transcode_call | amrwb_be_pcmu | TLS | caller | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/freeswitch/endpoint/amr_transcode_call/amrwb_be_pcmu/TLS/caller.log` |
| PASS | freeswitch | endpoint | amr_transcode_call | amrwb_be_pcmu | TLS | callee | 5s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/freeswitch/endpoint/amr_transcode_call/amrwb_be_pcmu/TLS/callee.log` |
