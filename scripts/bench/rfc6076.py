"""RFC 6076 end-to-end SIP metrics from the caller's per-call records.

Ported from the E74 harness (rfc6076.py). Input rows are the caller's
`call_ended` fields (events.jsonl) or the observation of report.json `calls`.
The caller adds an `rfc6076` record per call with wire instants taken at its
transport boundary; a row without one has only `sip_status`, so SRD/SDD are
reported as unavailable rather than approximated.

Metric definitions (RFC 6076, section numbers in the output):
  SER  §4.6  INVITEs with a 200 / (INVITEs - INVITEs with 3xx) x 100
  SEER §4.7  INVITEs with 200, 480, 486, 600 or 603 / (INVITEs - 3xx) x 100
  ISA  §4.8  INVITEs ending 408, 500, 503, 504 (or no final response:
             a client transaction timeout is reported as 408) / INVITEs x 100
  SCR  §4.9  established sessions ended by a BYE with a 2xx / INVITEs x 100
  SRD  §4.3  INVITE to first non-100 provisional (else the final); successful
             and failed SRD are reported separately and never combined
  SDD  §4.4  BYE to its 2xx, only for BYEs the caller sent; failed BYE
             transactions are excluded
  SDT  §4.5  200 OK to BYE; the RFC reports the average, percentiles added
RRD and IRA (§4.1-4.2) need REGISTER, which the DUT does not serve.

Distributions report count, mean, p50, p95, p99, p99.9 and max (nearest
rank). Standard library only.
"""
import math

SEER_CODES = (200, 480, 486, 600, 603)
ISA_CODES = (408, 500, 503, 504)
QUANTILES = (('p50', .5), ('p95', .95), ('p99', .99), ('p99_9', .999))


def distribution(values):
    """count/mean/p50/p95/p99/p99.9/max by nearest rank; None fields when empty."""
    ordered = sorted(value for value in values if value is not None)
    row = {'count': len(ordered), 'mean': sum(ordered) / len(ordered) if ordered else None}
    for name, q in QUANTILES:
        row[name] = ordered[min(len(ordered) - 1, max(0, math.ceil(q * len(ordered)) - 1))] if ordered else None
    row['max'] = ordered[-1] if ordered else None
    return row


def final_status(call):
    """Final response to the initial INVITE: the wire record when present,
    else the caller's typed status (200 when answered)."""
    record = call.get('rfc6076') or {}
    if record.get('final_status') is not None:
        return record['final_status']
    return call.get('sip_status')


def _pct(numerator, denominator):
    return 100.0 * numerator / denominator if denominator else None


def metrics(calls):
    """RFC 6076 metrics over terminal call rows (one INVITE per call: trials
    never retry a 503)."""
    calls = [call for call in calls if call is not None]
    invites = len(calls)
    statuses = [final_status(call) for call in calls]
    redirected = sum(status is not None and 300 <= status < 400 for status in statuses)
    established = sum(status is not None and 200 <= status < 300 for status in statuses)
    seer = sum(status in SEER_CODES for status in statuses)
    no_final = sum(status is None for status in statuses)
    isa = sum(status in ISA_CODES for status in statuses) + no_final
    records = [call.get('rfc6076') for call in calls if call.get('rfc6076')]
    # Completion: the complete wire record, else the caller's typed outcome
    # (a trace event dropped under load must not read as a failed BYE).
    completed = sum(rfc_session_ok(call) for call in calls)
    codes = {}
    for status in statuses:
        key = 'no_final_response' if status is None else str(status)
        codes[key] = codes.get(key, 0) + 1
    return {
        'basis': 'RFC 6076 §4.3-4.9 at the caller (originating UA); one INVITE per call; times in ms',
        'invites': invites,
        'final_status_counts': dict(sorted(codes.items())),
        'invites_with_3xx': redirected,
        'established': established,
        'ser_pct': _pct(established, invites - redirected),
        'seer_pct': _pct(seer, invites - redirected),
        'isa_count': isa,
        'isa_pct': _pct(isa, invites),
        'completed': completed,
        'scr_pct': _pct(completed, invites),
        'completion_source': 'rfc6076 wire trace when complete, else the caller outcome',
        'srd_successful_ms': distribution(r.get('srd_ms') for r in records if r.get('srd_kind') == 'successful'),
        'srd_failed_ms': distribution(r.get('srd_ms') for r in records if r.get('srd_kind') == 'failed'),
        'sdd_ms': distribution(r.get('sdd_ms') for r in records),
        'sdt_ms': distribution(r.get('sdt_ms') for r in records),
        'first_response_counts': _first_responses(records),
        'disconnect_failures': sum(disconnect_failure(call) for call in calls),
        'trace': {'records': len(records), 'calls_without_record': invites - len(records),
                  'incomplete_records': sum(not r.get('trace_complete') for r in records),
                  'invite_retransmissions': sum(max(0, (r.get('invite_transmissions') or 1) - 1) for r in records),
                  'available': bool(records)},
    }


def _first_responses(records):
    out = {}
    for record in records:
        status = record.get('first_response_status')
        key = 'none' if status is None else str(status)
        out[key] = out.get(key, 0) + 1
    return dict(sorted(out.items()))


def rfc_session_failure(call):
    """RFC 7501 §3.1.8 Session Attempt Failure: no 200 OK (3xx-6xx final or
    no final within the establishment threshold). Completion (§4.9 SCR) is
    scored separately by `rfc_session_ok`."""
    status = final_status(call)
    return status is None or not 200 <= status < 300


def rfc_attempt_ok(call):
    """The search criterion: RFC 7501 §3.1.8 counts Session Attempt Failures
    only (a 3xx-6xx final, or no final within the establishment threshold).
    A non-2xx to the caller's BYE is a disconnect failure, reported separately
    (`disconnect_failure`), not a failed attempt."""
    return not rfc_session_failure(call)


def disconnect_failure(call):
    """Established, and the caller's BYE got a final that is not 2xx (complete
    wire record; RFC 6076 §4.5 SDD)."""
    if rfc_session_failure(call):
        return False
    record = call.get('rfc6076') or {}
    status = record.get('bye_final_status')
    return bool(record.get('trace_complete')) and status is not None and not 200 <= status < 300


def rfc_session_ok(call):
    """Established (RFC 7501 §3.1.7) and, when the caller ended the session,
    the BYE got a 2xx (RFC 6076 §4.9); media is not part of this criterion."""
    if rfc_session_failure(call):
        return False
    record = call.get('rfc6076')
    if record and record.get('trace_complete'):
        return bool(record.get('completed'))
    # No or an incomplete wire record (a dropped trace event): the caller's own
    # typed outcome decides; for the hold and signaling-only call models a
    # pass includes the BYE's 2xx.
    return call.get('outcome') == 'pass' and call.get('sip_status') == 200
