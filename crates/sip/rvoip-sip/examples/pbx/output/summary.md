# PBX Interop Run Summary

- started_at_utc: 2026-08-11T19:25:46Z
- ended_at_utc: 2026-08-11T19:25:55Z
- duration_seconds: 9
- exit_status: 0
- output_root: /Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output
- environments: `environment-*.md`
- matrix: `matrix.tsv`

## Result

- total_cells: 2
- passed_cells: 2
- failed_cells: 0

## Matrix

| Status | Provider | API | Scenario | Transport | Role | Duration | Exit | Log |
|--------|----------|-----|----------|-----------|------|----------|------|-----|
| PASS | asterisk | endpoint | amr_call | UDP | callee | 9s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/asterisk/endpoint/amr_call/amrwb/UDP/callee.log` |
| PASS | asterisk | endpoint | amr_call | UDP | caller | 6s | 0 | `/Users/jonathan/Developer/rvoip/crates/sip/rvoip-sip/examples/pbx/output/asterisk/endpoint/amr_call/amrwb/UDP/caller.log` |
