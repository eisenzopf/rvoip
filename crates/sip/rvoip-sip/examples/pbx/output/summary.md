# PBX Interop Run Summary

- started_at_utc: 2026-08-12T06:38:22Z
- ended_at_utc: 2026-08-12T06:41:06Z
- duration_seconds: 164
- exit_status: 0
- output_root: /Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output
- environments: `environment-*.md`
- matrix: `matrix.tsv`

## Result

- total_cells: 96
- passed_cells: 96
- failed_cells: 0
- skipped_cells: 0

## Matrix

| Status | Provider | API | Scenario | Codec | Transport | Role | Duration | Exit | Log |
|--------|----------|-----|----------|-------|-----------|------|----------|------|-----|
| PASS | kamailio | endpoint | registration | default | UDP | registration | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/endpoint/registration/UDP/registration.log` |
| PASS | kamailio | endpoint | basic_call | default | UDP | callee | 7s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/endpoint/basic_call/UDP/callee.log` |
| PASS | kamailio | endpoint | basic_call | default | UDP | caller | 6s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/endpoint/basic_call/UDP/caller.log` |
| PASS | kamailio | analyzer | basic_call | default | UDP | analyze | 1s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/endpoint/basic_call/UDP/analyze.log` |
| PASS | kamailio | endpoint | amr_call | amrnb | UDP | caller | 3s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/endpoint/amr_call/amrnb/UDP/caller.log` |
| PASS | kamailio | endpoint | amr_call | amrnb | UDP | callee | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/endpoint/amr_call/amrnb/UDP/callee.log` |
| PASS | kamailio | analyzer | amr_call | amrnb | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/endpoint/amr_call/amrnb/UDP/analyze.log` |
| PASS | kamailio | endpoint | amr_call | amrwb | UDP | caller | 3s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/endpoint/amr_call/amrwb/UDP/caller.log` |
| PASS | kamailio | endpoint | amr_call | amrwb | UDP | callee | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/endpoint/amr_call/amrwb/UDP/callee.log` |
| PASS | kamailio | analyzer | amr_call | amrwb | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/endpoint/amr_call/amrwb/UDP/analyze.log` |
| PASS | kamailio | endpoint | amr_call | amrnb_be | UDP | callee | 5s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/endpoint/amr_call/amrnb_be/UDP/callee.log` |
| PASS | kamailio | endpoint | amr_call | amrnb_be | UDP | caller | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/endpoint/amr_call/amrnb_be/UDP/caller.log` |
| PASS | kamailio | analyzer | amr_call | amrnb_be | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/endpoint/amr_call/amrnb_be/UDP/analyze.log` |
| PASS | kamailio | endpoint | amr_call | amrwb_be | UDP | callee | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/endpoint/amr_call/amrwb_be/UDP/callee.log` |
| PASS | kamailio | endpoint | amr_call | amrwb_be | UDP | caller | 3s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/endpoint/amr_call/amrwb_be/UDP/caller.log` |
| PASS | kamailio | analyzer | amr_call | amrwb_be | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/endpoint/amr_call/amrwb_be/UDP/analyze.log` |
| PASS | kamailio | stream_peer | registration | default | UDP | registration | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/stream_peer/registration/UDP/registration.log` |
| PASS | kamailio | stream_peer | basic_call | default | UDP | caller | 5s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/stream_peer/basic_call/UDP/caller.log` |
| PASS | kamailio | stream_peer | basic_call | default | UDP | callee | 6s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/stream_peer/basic_call/UDP/callee.log` |
| PASS | kamailio | analyzer | basic_call | default | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/stream_peer/basic_call/UDP/analyze.log` |
| PASS | kamailio | stream_peer | amr_call | amrnb | UDP | callee | 5s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/stream_peer/amr_call/amrnb/UDP/callee.log` |
| PASS | kamailio | stream_peer | amr_call | amrnb | UDP | caller | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/stream_peer/amr_call/amrnb/UDP/caller.log` |
| PASS | kamailio | analyzer | amr_call | amrnb | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/stream_peer/amr_call/amrnb/UDP/analyze.log` |
| PASS | kamailio | stream_peer | amr_call | amrwb | UDP | callee | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/stream_peer/amr_call/amrwb/UDP/callee.log` |
| PASS | kamailio | stream_peer | amr_call | amrwb | UDP | caller | 3s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/stream_peer/amr_call/amrwb/UDP/caller.log` |
| PASS | kamailio | analyzer | amr_call | amrwb | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/stream_peer/amr_call/amrwb/UDP/analyze.log` |
| PASS | kamailio | stream_peer | amr_call | amrnb_be | UDP | callee | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/stream_peer/amr_call/amrnb_be/UDP/callee.log` |
| PASS | kamailio | stream_peer | amr_call | amrnb_be | UDP | caller | 3s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/stream_peer/amr_call/amrnb_be/UDP/caller.log` |
| PASS | kamailio | analyzer | amr_call | amrnb_be | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/stream_peer/amr_call/amrnb_be/UDP/analyze.log` |
| PASS | kamailio | stream_peer | amr_call | amrwb_be | UDP | caller | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/stream_peer/amr_call/amrwb_be/UDP/caller.log` |
| PASS | kamailio | stream_peer | amr_call | amrwb_be | UDP | callee | 5s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/stream_peer/amr_call/amrwb_be/UDP/callee.log` |
| PASS | kamailio | analyzer | amr_call | amrwb_be | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/stream_peer/amr_call/amrwb_be/UDP/analyze.log` |
| PASS | kamailio | callback | registration | default | UDP | registration | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/callback/registration/UDP/registration.log` |
| PASS | kamailio | callback | basic_call | default | UDP | callee | 6s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/callback/basic_call/UDP/callee.log` |
| PASS | kamailio | callback | basic_call | default | UDP | caller | 5s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/callback/basic_call/UDP/caller.log` |
| PASS | kamailio | analyzer | basic_call | default | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/callback/basic_call/UDP/analyze.log` |
| PASS | kamailio | callback | amr_call | amrnb | UDP | caller | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/callback/amr_call/amrnb/UDP/caller.log` |
| PASS | kamailio | callback | amr_call | amrnb | UDP | callee | 5s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/callback/amr_call/amrnb/UDP/callee.log` |
| PASS | kamailio | analyzer | amr_call | amrnb | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/callback/amr_call/amrnb/UDP/analyze.log` |
| PASS | kamailio | callback | amr_call | amrwb | UDP | callee | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/callback/amr_call/amrwb/UDP/callee.log` |
| PASS | kamailio | callback | amr_call | amrwb | UDP | caller | 3s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/callback/amr_call/amrwb/UDP/caller.log` |
| PASS | kamailio | analyzer | amr_call | amrwb | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/callback/amr_call/amrwb/UDP/analyze.log` |
| PASS | kamailio | callback | amr_call | amrnb_be | UDP | callee | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/callback/amr_call/amrnb_be/UDP/callee.log` |
| PASS | kamailio | callback | amr_call | amrnb_be | UDP | caller | 3s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/callback/amr_call/amrnb_be/UDP/caller.log` |
| PASS | kamailio | analyzer | amr_call | amrnb_be | UDP | analyze | 1s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/callback/amr_call/amrnb_be/UDP/analyze.log` |
| PASS | kamailio | callback | amr_call | amrwb_be | UDP | callee | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/callback/amr_call/amrwb_be/UDP/callee.log` |
| PASS | kamailio | callback | amr_call | amrwb_be | UDP | caller | 3s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/callback/amr_call/amrwb_be/UDP/caller.log` |
| PASS | kamailio | analyzer | amr_call | amrwb_be | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/kamailio/callback/amr_call/amrwb_be/UDP/analyze.log` |
| PASS | opensips | endpoint | registration | default | UDP | registration | 2s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/endpoint/registration/UDP/registration.log` |
| PASS | opensips | endpoint | basic_call | default | UDP | caller | 6s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/endpoint/basic_call/UDP/caller.log` |
| PASS | opensips | endpoint | basic_call | default | UDP | callee | 7s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/endpoint/basic_call/UDP/callee.log` |
| PASS | opensips | analyzer | basic_call | default | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/endpoint/basic_call/UDP/analyze.log` |
| PASS | opensips | endpoint | amr_call | amrnb | UDP | caller | 3s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/endpoint/amr_call/amrnb/UDP/caller.log` |
| PASS | opensips | endpoint | amr_call | amrnb | UDP | callee | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/endpoint/amr_call/amrnb/UDP/callee.log` |
| PASS | opensips | analyzer | amr_call | amrnb | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/endpoint/amr_call/amrnb/UDP/analyze.log` |
| PASS | opensips | endpoint | amr_call | amrwb | UDP | caller | 3s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/endpoint/amr_call/amrwb/UDP/caller.log` |
| PASS | opensips | endpoint | amr_call | amrwb | UDP | callee | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/endpoint/amr_call/amrwb/UDP/callee.log` |
| PASS | opensips | analyzer | amr_call | amrwb | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/endpoint/amr_call/amrwb/UDP/analyze.log` |
| PASS | opensips | endpoint | amr_call | amrnb_be | UDP | caller | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/endpoint/amr_call/amrnb_be/UDP/caller.log` |
| PASS | opensips | endpoint | amr_call | amrnb_be | UDP | callee | 5s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/endpoint/amr_call/amrnb_be/UDP/callee.log` |
| PASS | opensips | analyzer | amr_call | amrnb_be | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/endpoint/amr_call/amrnb_be/UDP/analyze.log` |
| PASS | opensips | endpoint | amr_call | amrwb_be | UDP | caller | 3s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/endpoint/amr_call/amrwb_be/UDP/caller.log` |
| PASS | opensips | endpoint | amr_call | amrwb_be | UDP | callee | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/endpoint/amr_call/amrwb_be/UDP/callee.log` |
| PASS | opensips | analyzer | amr_call | amrwb_be | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/endpoint/amr_call/amrwb_be/UDP/analyze.log` |
| PASS | opensips | stream_peer | registration | default | UDP | registration | 2s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/stream_peer/registration/UDP/registration.log` |
| PASS | opensips | stream_peer | basic_call | default | UDP | caller | 6s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/stream_peer/basic_call/UDP/caller.log` |
| PASS | opensips | stream_peer | basic_call | default | UDP | callee | 7s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/stream_peer/basic_call/UDP/callee.log` |
| PASS | opensips | analyzer | basic_call | default | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/stream_peer/basic_call/UDP/analyze.log` |
| PASS | opensips | stream_peer | amr_call | amrnb | UDP | callee | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/stream_peer/amr_call/amrnb/UDP/callee.log` |
| PASS | opensips | stream_peer | amr_call | amrnb | UDP | caller | 3s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/stream_peer/amr_call/amrnb/UDP/caller.log` |
| PASS | opensips | analyzer | amr_call | amrnb | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/stream_peer/amr_call/amrnb/UDP/analyze.log` |
| PASS | opensips | stream_peer | amr_call | amrwb | UDP | caller | 3s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/stream_peer/amr_call/amrwb/UDP/caller.log` |
| PASS | opensips | stream_peer | amr_call | amrwb | UDP | callee | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/stream_peer/amr_call/amrwb/UDP/callee.log` |
| PASS | opensips | analyzer | amr_call | amrwb | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/stream_peer/amr_call/amrwb/UDP/analyze.log` |
| PASS | opensips | stream_peer | amr_call | amrnb_be | UDP | callee | 5s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/stream_peer/amr_call/amrnb_be/UDP/callee.log` |
| PASS | opensips | stream_peer | amr_call | amrnb_be | UDP | caller | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/stream_peer/amr_call/amrnb_be/UDP/caller.log` |
| PASS | opensips | analyzer | amr_call | amrnb_be | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/stream_peer/amr_call/amrnb_be/UDP/analyze.log` |
| PASS | opensips | stream_peer | amr_call | amrwb_be | UDP | caller | 3s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/stream_peer/amr_call/amrwb_be/UDP/caller.log` |
| PASS | opensips | stream_peer | amr_call | amrwb_be | UDP | callee | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/stream_peer/amr_call/amrwb_be/UDP/callee.log` |
| PASS | opensips | analyzer | amr_call | amrwb_be | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/stream_peer/amr_call/amrwb_be/UDP/analyze.log` |
| PASS | opensips | callback | registration | default | UDP | registration | 2s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/callback/registration/UDP/registration.log` |
| PASS | opensips | callback | basic_call | default | UDP | caller | 6s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/callback/basic_call/UDP/caller.log` |
| PASS | opensips | callback | basic_call | default | UDP | callee | 7s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/callback/basic_call/UDP/callee.log` |
| PASS | opensips | analyzer | basic_call | default | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/callback/basic_call/UDP/analyze.log` |
| PASS | opensips | callback | amr_call | amrnb | UDP | callee | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/callback/amr_call/amrnb/UDP/callee.log` |
| PASS | opensips | callback | amr_call | amrnb | UDP | caller | 3s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/callback/amr_call/amrnb/UDP/caller.log` |
| PASS | opensips | analyzer | amr_call | amrnb | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/callback/amr_call/amrnb/UDP/analyze.log` |
| PASS | opensips | callback | amr_call | amrwb | UDP | caller | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/callback/amr_call/amrwb/UDP/caller.log` |
| PASS | opensips | callback | amr_call | amrwb | UDP | callee | 5s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/callback/amr_call/amrwb/UDP/callee.log` |
| PASS | opensips | analyzer | amr_call | amrwb | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/callback/amr_call/amrwb/UDP/analyze.log` |
| PASS | opensips | callback | amr_call | amrnb_be | UDP | callee | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/callback/amr_call/amrnb_be/UDP/callee.log` |
| PASS | opensips | callback | amr_call | amrnb_be | UDP | caller | 3s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/callback/amr_call/amrnb_be/UDP/caller.log` |
| PASS | opensips | analyzer | amr_call | amrnb_be | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/callback/amr_call/amrnb_be/UDP/analyze.log` |
| PASS | opensips | callback | amr_call | amrwb_be | UDP | caller | 4s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/callback/amr_call/amrwb_be/UDP/caller.log` |
| PASS | opensips | callback | amr_call | amrwb_be | UDP | callee | 5s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/callback/amr_call/amrwb_be/UDP/callee.log` |
| PASS | opensips | analyzer | amr_call | amrwb_be | UDP | analyze | 0s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/opensips/callback/amr_call/amrwb_be/UDP/analyze.log` |
