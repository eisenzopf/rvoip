#!/usr/bin/env python3
"""RFC 7502-format benchmark report (.md and .html) from lane evidence.

Ported from the E74 harness (rfc_report.py); the section builders are shared
with rfc_report_ietf.py (the IETF edition rendered by the minor profile).

usage: rfc_report.py SPEC.json OUT_DIR           writes OUT_DIR/rfc7502-report.md and .html
       rfc_report.py --template OUT_DIR          the same structure, every value "pending"

SPEC.json:
  {"title", "report_date", "status", "profile": "minor|full|baseline",
   "evidence_root": "<local copy of bench/<lane-run-id>/>",
   "evidence_uri": "s3://<evidence-bucket>/bench/<lane-run-id>/",
   "dut": {"name": "rvoip-bench-uas", "version", "git_sha", "binary_sha256", "instance_type", "vcpu",
           "physical_cores", "config": {...dut.json fields...}, "notes"},
   "testbed": {"caller_hosts", "caller_type", "caller_vcpu", "sources_per_host", "connections_per_source",
               "caller_build", "caller_sha256", "ea_uas_build"},
   "environment": {"env_id", "region", "ami", "placement", ...},
   "rows": [{"row": "R1", "search": "R1/search.json", "trials": "R1/trials.jsonl"}, ...],
   "baseline": {"R1": {"search": "baseline/R1/search.json"} | {"result_sps": 9426, "tester_limited": false,
                                                                 "source": "rates.json"}},
   "overload": {"row": "R4", "trial": "R4/t00-overload"},             (optional)
   "summary": [...], "findings": [...], "deviations": [...], "assumptions": [...]}

A search state is rfc7502.Search JSON (search.json); each recorded trial's
detail.run names its trial directory (relative to evidence_root), whose
merged/summary.json (summarize_trial.py) supplies the RFC 6076 metrics and the
resource figures. Missing evidence renders as "pending"; an empty
distribution (no such calls) renders as "-". The report never invents a
number. Standard library only.
"""
import datetime
import html
import json
import math
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import rfc7502  # noqa: E402

PENDING = 'pending'
EMPTY = '-'
VALIDITY = rfc7502.TESTER_CAP_FACTOR
# Row definitions (RFC7502_BENCHMARK.md §1; E74 numbering).
ROWS = {
    'R1': {'label': 'R1 Session Establishment Rate, no media', 'ref': 'RFC 7502 §6.2 + §6.5', 'transport': 'TLS',
           'media': 'none', 'hold': '0 s'},
    'R2': {'label': 'R2 Session Establishment Rate, with media', 'ref': 'RFC 7502 §6.4 + §6.5', 'transport': 'TLS',
           'media': 'SDES-SRTP G.711 20 ms, rtcp-mux, echoed by the DUT', 'hold': '10 s'},
    'R4': {'label': 'R4 Overload and recovery (0.5/1.5/2 x C, admission on)', 'ref': 'RFC 7339 behaviour (not an RFC 7502 benchmark)',
           'transport': 'TLS', 'media': 'SDES-SRTP G.711 20 ms', 'hold': '10 s'},
    'R5': {'label': 'R5 Session Establishment Rate, no media', 'ref': 'RFC 7502 §6.2', 'transport': 'UDP',
           'media': 'none', 'hold': '0 s'},
    'R6': {'label': 'R6 Session Establishment Rate, with media', 'ref': 'RFC 7502 §6.4', 'transport': 'UDP',
           'media': 'RTP G.711 20 ms, rtcp-mux, echoed by the DUT', 'hold': '10 s'},
}
DEFAULT_ASSUMPTIONS = [
    'The DUT is rvoip-bench-uas: a terminating SIP UAS and media endpoint built on rvoip. It is not a proxy, B2BUA or '
    'registrar; it answers every INVITE with 200 OK and, with media, echoes every received RTP packet to its source '
    '(SRTP: decrypted and re-encrypted with the answer key) so the caller validates its own 440 Hz tone.',
    'The Tester is rvoip-loadgen: caller hosts each acting as many Emulated Agents (UACs), every host offering an even '
    'share of every stage on a shared timeline (start barrier; start skew <= 2 s).',
    'Clocks: every RFC 6076 interval is measured on one caller host (t1 and t4 on the same clock), so no cross-host '
    'offset applies (RFC 6076 §3); resolution is 1 ms from the SIP stack\'s transport trace.',
    'Establishment Threshold Time: 10 s, the caller\'s setup bound. This is stricter than the RFC 7501 §3.3.2 '
    'recommendation (Timer B = 64 x T1 = 32 s): a call without a 200 OK within 10 s is a Session Attempt Failure.',
    'Media Packet Size: G.711 at 20 ms = 160 payload bytes per RTP packet (172 bytes RTP; 182 bytes SRTP with the '
    '80-bit authentication tag). RTCP is multiplexed on the RTP port (rtcp-mux, RFC 5761).',
]
DEFAULT_DEVIATIONS = [
    ('Topology', 'RFC 7501 §1 and RFC 7502 §3 scope the benchmarks to intermediaries (proxy, B2BUA, SBC, registrar) with '
     'EAs on both sides. The DUT terminates calls, so the topology is EA (UAC) -> DUT (UAS); media terminates on the DUT '
     '(RFC 7502 Figure 2 with the DUT as the far end). No second EA, no forwarding.'),
    ('Media relay field', 'RFC 7502 §5.2 asks whether the DUT is a media relay. It is not: it is the media endpoint '
     '(it echoes media). The "no media" rows still negotiate SDP; the DUT allocates no RTP socket for them.'),
    ('Warm-up', 'Each trial starts with an unscored warm-up: 30 s at half the trial rate, or 120 s ramped 25/50/75/100 % '
     'of the rate when admission is on. RFC 7502 defines no warm-up.'),
    ('Fresh process per trial', 'Every trial starts a fresh DUT process and drains it fully (true zero) before the next. '
     'RFC 7502 leaves inter-trial conditions open.'),
    ('Trial length', 'The scored stage lasts max(N / r, 180 s) with N = 50,000 (the sustained rule: RFC 7501 §3.4.1 '
     'defines the rate over "an extended, pre-defined period"); the N actually attempted is reported per trial.'),
    ('Rate quantum', 'Trial rates are floored to a multiple of 4 sessions/s, and every caller host\'s share of every '
     'stage times its seconds is a whole number of calls. RFC 7502 floors to 1.'),
    ('Tester cap', 'The search cannot exceed r_max = 0.67 x the §6.1 testbed baseline of the same row; a result at the '
     'cap, or above 0.67 x the baseline, is reported as R >= value (tester-limited), never as R.'),
    ('Invalid trials', 'A trial with tester-side defects (calls without terminal evidence, undispatched or '
     'generator-skipped calls, a caller outside its validity bounds before the DUT\'s first backpressure) is repeated '
     'at the same rate and never scored as a DUT failure; three invalid trials in a row stop the search.'),
    ('Establishment threshold', '10 s instead of the RFC 7501 §3.3.2 recommendation of 32 s (Timer B).'),
    ('503 with Retry-After', 'A 503 is a DUT Session Attempt Failure, never a tester fault. With admission on the DUT '
     'answers excess INVITEs 503 with Retry-After; RFC 7339 §5.10.2 asks overloaded servers to reject '
     'non-participating clients without Retry-After, and the RFC 7339 oc Via parameters are not implemented.'),
    ('Success criterion', 'Session Attempt Failures only (RFC 7501 §3.1.8). A non-2xx to the caller\'s BYE is a '
     'disconnect failure: counted and reported (RFC 6076 section), never a failed attempt. Media quality is reported, '
     'not scored.'),
    ('Percentiles', 'RFC 6076 defines no percentiles (SDT is an average). This report adds count, mean, p50, p95, p99, '
     'p99.9 and max (nearest rank) for every interval.'),
    ('Registration', 'RFC 7502 §6.7-6.8 and RFC 6076 RRD/IRA are not applicable: the DUT is not a registrar.'),
    ('Algorithm variant', 'Searches marked "bisect" use a declared accelerator (bracket with the RFC weights, bisect to '
     '2 % resolution, then confirm the lower bound) instead of the exact §4.10 procedure.'),
]
TERMS = [
    'Session Attempt (RFC 7501 §3.1.6): an INVITE sent by the Emulated Agent (EA) to establish a session.',
    'Session Establishment (§3.1.7): the INVITE answered with 200 OK and acknowledged (ACK) within the establishment '
    'threshold time.',
    'Session Attempt Failure (§3.1.8): an attempt that is not established within the threshold (a 3xx-6xx final '
    'response or no final response).',
    'Session Establishment Rate R (§3.4.1 / RFC 7502 §4.10): the highest rate at which every attempt of a trial '
    '(N >= 50,000, scored stage >= 180 s) succeeds, found by the §4.10 search.',
    'Session Duration (§3.3.3): the hold before the EA sends BYE: 0 s without media, 10 s with media.',
    'Associated media (§3.3.1): G.711 RTP (UDP) or SRTP (TLS) for the session duration, echoed by the DUT.',
    'Testbed baseline (RFC 7502 §6.1): the same search with the EA talking to an EA in UAS mode (ea-uas, no DUT), which '
    'bounds what the tester can measure.',
]


# ---------------------------------------------------------------- evidence

def _load(path):
    try:
        return json.loads(Path(path).read_text())
    except (OSError, ValueError, TypeError):
        return None


class Evidence:
    """Search states and trial summaries under evidence_root."""

    def __init__(self, spec):
        self.root = Path(spec.get('evidence_root') or '.')
        self._summaries = {}

    def path(self, value):
        if not value:
            return None
        p = Path(value)
        return p if p.is_absolute() else self.root / p

    def summary(self, run):
        """merged/summary.json of a trial directory (relative to evidence_root), or None."""
        if not run:
            return None
        if run not in self._summaries:
            base = self.path(run)
            found = _load(base / 'merged' / 'summary.json')
            if found is None and base.name == 'summary.json':
                found = _load(base)
            self._summaries[run] = found
        return self._summaries[run]

    def search(self, value):
        """{state, result (Search.summary), trials, final_trial, summary} or None when the state is missing."""
        state = _load(self.path(value)) if value else None
        if not state or 'trials' not in state:
            return None
        result = rfc7502.Search.from_json(json.dumps(state)).summary()
        trials = state.get('trials') or []
        cap = result.get('result_sps')
        passing = [t for t in trials if t.get('outcome') == 'pass' and (cap is None or t['rate'] <= cap)]
        final = max(passing, key=lambda t: (t['rate'], t['index'])) if passing else None
        summary = self.summary((final.get('detail') or {}).get('run')) if final else None
        return {'state': state, 'result': result, 'trials': trials, 'final_trial': final, 'summary': summary}


# ---------------------------------------------------------------- formatting

def fmt(value, digits=1, unit=''):
    if value is None:
        return PENDING
    if isinstance(value, bool):
        return 'yes' if value else 'no'
    if isinstance(value, int):
        return '{:,}'.format(value) + unit
    if isinstance(value, float):
        if math.isnan(value):
            return PENDING
        if value.is_integer() and digits == 0:
            return '{:,}'.format(int(value)) + unit
        return ('{:,.%df}' % digits).format(value) + unit
    return str(value) + unit


def pct(value):
    return PENDING if value is None else '%.4f%%' % value


def dist_cells(d, digits=1):
    """p50/p95/p99/p99.9/max cells; '-' for an empty distribution, pending when the distribution is missing."""
    if d and d.get('count') == 0:
        return [EMPTY] * 5
    d = d or {}
    return [fmt(d.get(k), digits) for k in ('p50', 'p95', 'p99', 'p99_9', 'max')]


def dut_label(spec):
    dut = spec.get('dut') or {}
    name = dut.get('name') or 'rvoip-bench-uas'
    sha = dut.get('git_sha')
    return '%s (rvoip %s)' % (name, str(sha)[:12]) if sha else name


class Doc:
    """One report model rendered to Markdown and HTML."""

    def __init__(self):
        self.parts = []

    def h(self, level, text):
        self.parts.append(('h', level, text))

    def p(self, text):
        self.parts.append(('p', text))

    def ul(self, items):
        self.parts.append(('ul', list(items)))

    def table(self, header, rows, caption=None):
        self.parts.append(('table', header, [list(map(str, r)) for r in rows], caption))

    def markdown(self):
        out = []
        for part in self.parts:
            if part[0] == 'h':
                out.append('#' * part[1] + ' ' + part[2] + '\n')
            elif part[0] == 'p':
                out.append(part[1] + '\n')
            elif part[0] == 'ul':
                out.append('\n'.join('- ' + item for item in part[1]) + '\n')
            else:
                _, header, rows, caption = part
                if caption:
                    out.append('*' + caption + '*\n')
                esc = lambda cell: str(cell).replace('|', '\\|')  # noqa: E731
                out.append('| ' + ' | '.join(map(esc, header)) + ' |')
                out.append('|' + '|'.join('---' for _ in header) + '|')
                for row in rows:
                    out.append('| ' + ' | '.join(map(esc, row)) + ' |')
                out.append('')
        return '\n'.join(out).rstrip() + '\n'

    def html(self, title):
        body = []
        for part in self.parts:
            if part[0] == 'h':
                body.append('<h%d>%s</h%d>' % (part[1], html.escape(part[2]), part[1]))
            elif part[0] == 'p':
                body.append('<p>%s</p>' % html.escape(part[1]))
            elif part[0] == 'ul':
                body.append('<ul>%s</ul>' % ''.join('<li>%s</li>' % html.escape(i) for i in part[1]))
            else:
                _, header, rows, caption = part
                cells = ''.join('<tr>%s</tr>' % ''.join(
                    '<td class="%s">%s</td>' % ('pending' if PENDING in c else '', html.escape(c)) for c in row) for row in rows)
                body.append('<div class="table-wrap"><table>%s<thead><tr>%s</tr></thead><tbody>%s</tbody></table></div>' % (
                    '<caption>%s</caption>' % html.escape(caption) if caption else '',
                    ''.join('<th>%s</th>' % html.escape(h) for h in header), cells))
        return HTML_PAGE.replace('{title}', html.escape(title)).replace('{body}', '\n'.join(body))


HTML_PAGE = """<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
:root{--bg:#ffffff;--fg:#1d1d1f;--muted:#5f6368;--line:#d9dce1;--head:#f3f4f6;--accent:#2f5bd3;--pending:#9a6700}
@media (prefers-color-scheme: dark){:root:not([data-theme="light"]){--bg:#121417;--fg:#e8eaed;--muted:#a0a4ab;--line:#30343a;--head:#1c1f24;--accent:#8ab4f8;--pending:#e3b341}}
:root[data-theme="dark"]{--bg:#121417;--fg:#e8eaed;--muted:#a0a4ab;--line:#30343a;--head:#1c1f24;--accent:#8ab4f8;--pending:#e3b341}
body{background:var(--bg);color:var(--fg);font:15px/1.55 system-ui,-apple-system,"Segoe UI",sans-serif;margin:0;padding:24px 16px}
main{max-width:1100px;margin:0 auto}
h1{font-size:1.6rem;margin:.2em 0 .6em}h2{font-size:1.2rem;margin:1.8em 0 .5em;border-bottom:1px solid var(--line);padding-bottom:.2em}
h3{font-size:1.02rem;margin:1.3em 0 .4em}p,li{color:var(--fg)}caption{caption-side:top;text-align:left;color:var(--muted);padding:4px 0}
.table-wrap{overflow-x:auto;margin:.4em 0 1em}table{border-collapse:collapse;font-variant-numeric:tabular-nums;min-width:100%}
th,td{border:1px solid var(--line);padding:5px 8px;text-align:left;vertical-align:top;font-size:.9rem}th{background:var(--head)}
td.pending{color:var(--pending)}
</style></head><body><main>
{body}
</main></body></html>
"""


# ---------------------------------------------------------------- readings

def trial_rfc6076(summary):
    """RFC 6076 metrics of the scored trial-* stages (summary.rfc6076.scored)."""
    return ((summary or {}).get('rfc6076') or {}).get('scored')


def baseline_value(ctx, row):
    """(value, lower_bound, note) of the row's §6.1 testbed baseline.

    A baseline search that stopped on consecutive invalid trials (the tester's own limit) reads as its highest
    passing rate, a lower bound. A media baseline counts only if the far end echoed RTP (media_echo_observed)."""
    item = (ctx['spec'].get('baseline') or {}).get(row)
    if not item:
        return None, False, None
    if item.get('search'):
        found = ctx['ev'].search(item['search'])
        if not found:
            return None, False, None
        result = found['result']
        if ROWS.get(row, {}).get('media', 'none') != 'none' and found.get('summary') is not None:
            if (found['summary'].get('checks') or {}).get('media_echo_observed') is False:
                return None, False, 'void: ea-uas did not echo media in the final trial'
        value, limited = result.get('result_sps'), bool(result.get('tester_limited'))
        if value is None and result.get('highest_passing_sps') and 'invalid' in str(result.get('stop_reason', '')):
            return result['highest_passing_sps'], True, 'stopped on consecutive caller-invalid trials: lower bound'
        return value, limited, item['search']
    return item.get('result_sps'), bool(item.get('tester_limited')), item.get('source')


def classify(ctx, row, found):
    """(cell, binding) of a row's R under the 0.67 x baseline validity rule."""
    if not found:
        return PENDING, PENDING
    result = found['result']
    value = result.get('result_sps')
    if value is None:
        if not found['trials']:
            return PENDING, PENDING
        return ('not converged: passes up to %s, fails from %s' % (fmt(result.get('highest_passing_sps')),
                                                                   fmt(result.get('lowest_failing_sps'))),
                result.get('stop_reason') or 'search running')
    base, _, _ = baseline_value(ctx, row)
    if result.get('tester_limited'):
        return '>= %s sps' % fmt(value), 'tester (search reached r_max %s)' % fmt((result.get('parameters') or {}).get('r_max'))
    if base is None:
        return '%s sps' % fmt(value), 'DUT (baseline pending)'
    if value > VALIDITY * base:
        return '>= %s sps' % fmt(value), 'tester-limited (R > %.2f x baseline %s)' % (VALIDITY, fmt(base))
    return '%s sps' % fmt(value), 'DUT'


def load_of(summary):
    """Caller and DUT load of one trial summary (None fields when absent)."""
    summary = summary or {}
    validity = summary.get('caller_validity') or {}
    resource = summary.get('resource') or {}
    hosts = [row.get('host_peak_cpu_pct') for role, row in resource.items() if str(role).startswith('caller') and row]
    dut = summary.get('dut') or {}
    generator = summary.get('generator') or {}
    sources = (validity.get('source_pools') or {}).get('sources')
    per = generator.get('connections_per_source')
    return {'sources': sources, 'flows': sources * per if sources and per else generator.get('endpoints'),
            'process_cores': validity.get('process_peak_cores'), 'core_limit': validity.get('process_core_limit'),
            'hot_thread': validity.get('thread_peak_cores'),
            'host_cpu': max([h for h in hosts if h is not None], default=None),
            'dut_cores': dut.get('peak_cpu_cores'), 'dut_host_cpu': dut.get('peak_host_cpu_pct'),
            'dut_rss': dut.get('peak_rss_bytes'), 'dut_active': dut.get('peak_active_calls')}


def failure_breakdown(ctx, found):
    """Final-status breakdown of the lowest failing trial above R (else the highest failing trial)."""
    trials = [t for t in (found or {}).get('trials') or [] if t.get('outcome') == 'fail']
    if not trials:
        return None
    value = (found['result'] or {}).get('result_sps')
    above = [t for t in trials if value is None or t['rate'] > value]
    worst = min(above, key=lambda t: t['rate']) if above else max(trials, key=lambda t: t['rate'])
    run = (worst.get('detail') or {}).get('run')
    metrics = trial_rfc6076(ctx['ev'].summary(run))
    statuses = (metrics or {}).get('final_status_counts') or (worst.get('detail') or {}).get('final_status_counts')
    return {'rate': worst['rate'], 'attempted': worst.get('attempted'), 'failed': worst.get('failed'),
            'statuses': statuses, 'run': run}


def context(spec, template=False):
    ev = Evidence(spec)
    rows = [r for r in spec.get('rows') or [] if r.get('row')]
    found = {r['row']: ev.search(r.get('search')) for r in rows}
    return {'spec': spec, 'ev': ev, 'rows': rows, 'found': found, 'template': template,
            'testbed': spec.get('testbed') or {}, 'dut': spec.get('dut') or {}}


def row_label(row):
    return ROWS.get(row, {}).get('label', row)


# ---------------------------------------------------------------- sections

def section_headline(doc, ctx, title):
    doc.h(2, title)
    rows = []
    for r in ctx['rows']:
        row = r['row']
        info = ROWS.get(row, {})
        found = ctx['found'][row]
        cell, binding = classify(ctx, row, found)
        base, lower, _ = baseline_value(ctx, row)
        result = (found or {}).get('result') or {}
        rows.append([row, info.get('transport', PENDING), info.get('media', PENDING), info.get('hold', PENDING),
                     cell, binding, (('>= ' if lower else '') + fmt(base) + ' sps') if base is not None else PENDING,
                     fmt((result.get('parameters') or {}).get('r_max')) if found else PENDING,
                     fmt(result.get('trials')) if found else PENDING])
    doc.table(['Row', 'Transport', 'Media', 'Hold', 'Session Establishment Rate R', 'Binding limit', '§6.1 baseline',
               'r_max (sps)', 'Trials'], rows,
              caption='sps = sessions per second. Each trial: unscored warm-up, then >= 180 s at the rate '
                      '(N = max(50,000, rate x 180)) with zero Session Attempt Failures (RFC 7501 §3.1.8); fresh DUT '
                      'process per trial. A result at r_max or above 0.67 x the §6.1 baseline reads ">= R" '
                      '(tester-limited). DUT: %s.' % dut_label(ctx['spec']))


def section_terms(doc, title):
    doc.h(2, title)
    doc.ul(TERMS)


def section_baseline(doc, ctx, title):
    doc.h(2, title)
    bed = ctx['testbed']
    doc.p('The EA (UAC) runs the same §4.10 search against ea-uas (an EA in UAS mode, no rvoip, no DUT) with the exact '
          'generator configuration of the DUT searches: %s caller hosts (%s), %s sources each, %s connections per '
          'source, caller build %s. r_max = %.2f x the baseline of the same row.' % (
              fmt(bed.get('caller_hosts')), bed.get('caller_type', PENDING), fmt(bed.get('sources_per_host')),
              fmt(bed.get('connections_per_source')), bed.get('caller_build', PENDING), VALIDITY))
    rows = []
    for r in ctx['rows']:
        row = r['row']
        value, lower, source = baseline_value(ctx, row)
        item = (ctx['spec'].get('baseline') or {}).get(row) or {}
        found = ctx['ev'].search(item.get('search')) if item.get('search') else None
        load = load_of((found or {}).get('summary')) if found else None
        rows.append([row, ROWS.get(row, {}).get('transport', PENDING), ROWS.get(row, {}).get('hold', PENDING),
                     (('>= ' if lower else '') + fmt(value) + ' sps') if value is not None else (source or PENDING),
                     fmt(rfc7502.tester_cap(value)) if value is not None else PENDING,
                     source or PENDING,
                     ('caller cores %s of %s, hottest thread %s, caller host CPU %s%%' % (
                         fmt(load['process_cores'], 2), fmt(load['core_limit']), fmt(load['hot_thread'], 2),
                         fmt(load['host_cpu'])) if load else 'recorded value')])
    doc.table(['Row', 'Transport', 'Hold', 'Baseline R', 'r_max (0.67 x)', 'Source', 'Generator load (final trial)'],
              rows or [[PENDING] * 7])


def section_setup(doc, ctx, title):
    doc.h(2, title)
    bed = ctx['testbed']
    for r in ctx['rows']:
        row = r['row']
        info = ROWS.get(row, {})
        found = ctx['found'][row]
        result = (found or {}).get('result') or {}
        params = result.get('parameters') or {}
        tls = info.get('transport') == 'TLS'
        media = info.get('media', 'none')
        doc.h(3, '%s (%s)' % (row_label(row), info.get('ref', '')))
        doc.table(['Field', 'Value'], [
            ['SIP transport protocol', info.get('transport', PENDING) + ' (same transport toward and from the DUT)'],
            ['DUT receives requests on', ('%s TLS connections (one per caller endpoint, client sockets bound to their source IP)'
                                          % fmt(_flows(bed))) if tls else '%s UDP 5-tuples' % fmt(_flows(bed))],
            ['Session duration', info.get('hold', PENDING)], ['Session ended by', 'the EA (BYE from the caller)'],
            ['Media streams per session', '0 (SDP offered, no RTP)' if media == 'none' else '1 (audio)'],
            ['Associated media protocol', 'none' if media == 'none' else ('SRTP (SDES)' if 'SRTP' in media else 'RTP')],
            ['Codec', 'none' if media == 'none' else 'G.711 PCMU'],
            ['Media packet size (audio)', 'n/a' if media == 'none' else '160 bytes payload per 20 ms (172 B RTP, 182 B SRTP)'],
            ['Establishment threshold time', '10 s (caller setup bound; stricter than Timer B = 32 s)'],
            ['TLS', 'rustls (TLS 1.2/1.3); the negotiated suite is in the trial evidence' if tls else 'n/a'],
            ['IPsec', 'not used'],
            ['Search', '%s (N %s, r0 %s, w %s, d %s, quantum %s, %s confirmations)' % (
                result.get('algorithm', PENDING), fmt(params.get('N')), fmt(params.get('r0')), fmt(params.get('w'), 2),
                fmt(params.get('d'), 2), fmt(params.get('quantum')), fmt(params.get('confirmations'))) if found else PENDING],
            ['Total sessions attempted', fmt(sum(t.get('attempted') or 0 for t in found['trials'])) if found else PENDING]])


def _flows(bed):
    try:
        return int(bed['caller_hosts']) * int(bed['sources_per_host']) * int(bed['connections_per_source'])
    except (KeyError, TypeError, ValueError):
        return None


def section_benchmarks(doc, ctx, title):
    doc.h(2, title)
    rows = []
    for r in ctx['rows']:
        row = r['row']
        found = ctx['found'][row]
        cell, binding = classify(ctx, row, found)
        result = (found or {}).get('result') or {}
        load = load_of((found or {}).get('summary'))
        rows.append([row_label(row), cell, binding, fmt(load['sources']), fmt(load['flows']),
                     result.get('stop_reason') or PENDING, fmt(result.get('trials')) if found else PENDING,
                     fmt(load['host_cpu']), fmt(load['process_cores'], 2), fmt(load['hot_thread'], 2),
                     fmt(load['dut_cores'], 2), fmt(load['dut_host_cpu']),
                     fmt(load['dut_rss'] / 2 ** 20, 0) if load['dut_rss'] is not None else PENDING,
                     'no (terminating media endpoint)'])
    doc.table(['Benchmark', 'R', 'Binding limit', 'Sources', 'Flows', 'Convergence', 'Trials', 'Caller host CPU % (peak)',
               'Caller process cores', 'Hottest caller thread (cores)', 'DUT peak cores', 'DUT host CPU %',
               'DUT peak RSS MiB', 'DUT acting as a media relay?'], rows or [[PENDING] * 14],
              caption='Resource use is the final passing trial at R. Registration benchmarks (RFC 7502 §5.3, test '
                      'cases 6.7 and 6.8): not applicable, the DUT is not a registrar.')


def section_rfc6076(doc, ctx, title):
    doc.h(2, title)
    ratios, dist, disconnect = [], [], []
    for r in ctx['rows']:
        row = r['row']
        found = ctx['found'][row]
        m = trial_rfc6076((found or {}).get('summary'))
        ratios.append([row_label(row), fmt((m or {}).get('invites')), pct((m or {}).get('ser_pct')),
                       pct((m or {}).get('seer_pct')), pct((m or {}).get('isa_pct')), pct((m or {}).get('scr_pct')),
                       fmt(((m or {}).get('sdt_ms') or {}).get('mean') / 1000.0, 3)
                       if ((m or {}).get('sdt_ms') or {}).get('mean') is not None else
                       (EMPTY if m and m['sdt_ms']['count'] == 0 else PENDING)])
        for name, block in (('SRD successful', 'srd_successful_ms'), ('SRD failed', 'srd_failed_ms'), ('SDD', 'sdd_ms'),
                            ('SDT', 'sdt_ms')):
            d = (m or {}).get(block)
            dist.append([row_label(row), name, fmt((d or {}).get('count'))] + dist_cells(d))
        trials = [t for t in (found or {}).get('trials') or [] if t.get('outcome') != 'invalid']
        offered = sum((t.get('detail') or {}).get('offered') or 0 for t in trials)
        failures = sum((t.get('detail') or {}).get('disconnect_failures') or 0 for t in trials)
        sdd = (m or {}).get('sdd_ms')
        disconnect.append([row_label(row), fmt(len(trials)) if trials else PENDING, fmt(offered) if trials else PENDING,
                           fmt(failures) if trials else PENDING, fmt(1e6 * failures / offered, 1) if offered else PENDING]
                          + dist_cells(sdd)[0:1] + dist_cells(sdd)[2:3])
    doc.table(['Benchmark', 'INVITEs', 'SER §4.6', 'SEER §4.7', 'ISA §4.8', 'SCR §4.9', 'SDT mean (s) §4.5'], ratios)
    doc.table(['Benchmark', 'Metric', 'Count', 'p50 ms', 'p95 ms', 'p99 ms', 'p99.9 ms', 'max ms'], dist,
              caption='Final passing trial at R, scored stage. SRD §4.3 (successful and failed never combined), SDD §4.4 '
                      '(BYEs sent by the caller), SDT §4.5. RRD/IRA §4.1-4.2: not applicable. "-" = no such calls.')
    doc.table(['Benchmark', 'Valid trials', 'Attempts', 'Disconnect failures', 'per 10^6', 'SDD p50 ms', 'SDD p99 ms'],
              disconnect,
              caption='Disconnect failures: established sessions whose caller BYE got a non-2xx final (e.g. 481), over all '
                      'valid trials; not Session Attempt Failures (RFC 7501 §3.1.8), so they never fail a trial.')


def section_latency_below(doc, ctx, title):
    doc.h(2, title)
    doc.p('The RFC 6076 tables report the final passing trial at R, where queueing near the limit dominates. The '
          'passing trials below R show the latency a carrier sees below the limit.')
    rows = []
    for r in ctx['rows']:
        row = r['row']
        found = ctx['found'][row]
        value = ((found or {}).get('result') or {}).get('result_sps')
        for t in sorted([t for t in (found or {}).get('trials') or [] if t.get('outcome') == 'pass'],
                        key=lambda t: (t['rate'], t['index'])):
            m = trial_rfc6076(ctx['ev'].summary((t.get('detail') or {}).get('run'))) or {}
            srd = dist_cells(m.get('srd_successful_ms'))
            sdd = dist_cells(m.get('sdd_ms'))
            rows.append([row, '%s sps%s' % (fmt(t['rate']), ' (%.0f%% of R)' % (100.0 * t['rate'] / value) if value else ''),
                         fmt(m.get('invites')), srd[0], srd[1], srd[2], srd[3], sdd[0], sdd[2]])
    doc.table(['Row', 'Trial rate', 'Sessions', 'SRD p50 ms', 'SRD p95 ms', 'SRD p99 ms', 'SRD p99.9 ms', 'SDD p50 ms',
               'SDD p99 ms'], rows or [[PENDING] * 9],
              caption='SRD = Session Request Delay (INVITE to 200 OK), SDD = Session Disconnect Delay (RFC 6076 §4.3-4.4), '
                      'successful sessions of each passing trial\'s scored stage.')


def section_failures(doc, ctx, title):
    doc.h(2, title)
    rows = []
    for r in ctx['rows']:
        row = r['row']
        found = ctx['found'][row]
        if not found:
            rows.append([row_label(row)] + [PENDING] * 5)
            continue
        b = failure_breakdown(ctx, found)
        if b:
            rows.append([row_label(row), fmt(b['rate']), fmt(b['attempted']), fmt(b['failed']),
                         json.dumps(b['statuses'], sort_keys=True) if b['statuses'] is not None else PENDING, b['run'] or PENDING])
        else:
            rows.append([row_label(row), EMPTY, EMPTY, EMPTY, 'no failing trial in this search', EMPTY])
    doc.table(['Benchmark', 'Rate (sps)', 'Attempted', 'Failed', 'Final status counts', 'Trial'], rows or [[PENDING] * 6],
              caption='The first failing rate above R. A 503 is a DUT failure (never a tester fault); '
                      '"no_final_response" is a setup without a final response within 10 s.')


def section_overload(doc, ctx, title):
    item = ctx['spec'].get('overload')
    if not item:
        return
    doc.h(2, title)
    summary = ctx['ev'].summary(item.get('trial'))
    rows = []
    for stage in (summary or {}).get('stage_summaries') or []:
        w = stage.get('whole_stage') or {}
        m = w.get('rfc6076') or {}
        rows.append([stage.get('name'), fmt(stage.get('requested_cps')), fmt(w.get('offered')), fmt(w.get('admitted')),
                     fmt(w.get('rejected_503')), pct(m.get('isa_pct')), dist_cells(w.get('setup_ms'))[2]])
    doc.table(['Stage', 'Offered sps', 'Attempts', 'Established', '503', 'ISA §4.8', 'Setup p99 ms'], rows or [[PENDING] * 7],
              caption='%s. Trial %s.' % (ROWS['R4']['label'], item.get('trial') or PENDING))


def section_traces(doc, ctx, title):
    doc.h(2, title)
    for r in ctx['rows']:
        row = r['row']
        found = ctx['found'][row]
        doc.h(3, row_label(row))
        if not found:
            doc.table(['Trial', 'Rate (sps)', 'Attempted n', 'Succeeded s', 'Outcome', 'Notes', 'Caller cores',
                       'Hottest caller thread', 'DUT peak cores', 'Trial'], [[PENDING] * 10])
            continue
        rows = []
        for t in found['trials']:
            detail = t.get('detail') or {}
            load = load_of(ctx['ev'].summary(detail.get('run')))
            notes = '; '.join(detail.get('invalid_reasons') or []) or '; '.join(detail.get('halt_reasons') or []) or EMPTY
            rows.append([t.get('index'), fmt(t.get('rate')), fmt(t.get('attempted')), fmt(t.get('succeeded')), t.get('outcome'),
                         notes, fmt(load['process_cores'], 2), fmt(load['hot_thread'], 2), fmt(load['dut_cores'], 2),
                         detail.get('run') or PENDING])
        doc.table(['Trial', 'Rate (sps)', 'Attempted n', 'Succeeded s', 'Outcome', 'Notes', 'Caller cores',
                   'Hottest caller thread', 'DUT peak cores', 'Trial'], rows or [[PENDING] * 10],
                  caption='Stop: %s' % (found['result'].get('stop_reason') or 'search not finished'))


def section_methodology(doc, ctx, title):
    doc.h(2, title)
    dut, bed, env = ctx['dut'], ctx['testbed'], ctx['spec'].get('environment') or {}
    rows = [['DUT', dut_label(ctx['spec'])]] + [['DUT ' + k, _cell(v)] for k, v in dut.items() if k not in ('name', 'git_sha')]
    rows += [['Testbed ' + k, _cell(v)] for k, v in bed.items()] + [['Environment ' + k, _cell(v)] for k, v in env.items()]
    doc.table(['Item', 'Value'], rows if len(rows) > 1 else rows + [[PENDING, PENDING]])
    doc.ul(DEFAULT_ASSUMPTIONS + list(ctx['spec'].get('assumptions') or []))
    doc.p('Topology (adapted from RFC 7502 Figure 2): Tester (EAs, UAC) --SIP over TLS or UDP, SRTP or RTP--> DUT (UAS, '
          'media endpoint that echoes media). There is no second EA and no forwarding.')


def _cell(value):
    return json.dumps(value, sort_keys=True) if isinstance(value, (dict, list)) else str(value)


def section_deviations(doc, ctx, title):
    doc.h(2, title)
    doc.table(['Area', 'Deviation'], DEFAULT_DEVIATIONS + [('Other', d) for d in ctx['spec'].get('deviations') or []])


def section_evidence(doc, ctx, title):
    doc.h(2, title)
    rows = []
    for r in ctx['rows']:
        found = ctx['found'][r['row']]
        rows.append([r['row'], r.get('search') or PENDING, r.get('trials') or EMPTY,
                     fmt(len(found['trials'])) if found else PENDING])
    for row, item in sorted((ctx['spec'].get('baseline') or {}).items()):
        rows.append(['baseline ' + row, item.get('search') or item.get('source') or PENDING, EMPTY, EMPTY])
    doc.table(['Row', 'Search state', 'Trials', 'Recorded trials'], rows or [[PENDING] * 4])
    doc.p('Raw evidence (per trial: plan.json, dut.tar.gz, caller-<i>.tar.gz with .sha256, merged/summary.json, '
          'verdict.json): %s' % (ctx['spec'].get('evidence_uri') or PENDING))


# ---------------------------------------------------------------- report

def build(spec, template=False):
    ctx = context(spec, template)
    doc = Doc()
    title = spec.get('title') or '%s: SIP benchmark (RFC 7501/7502, RFC 6076)' % dut_label(spec)
    doc.h(1, title)
    doc.p('Report date %s. Status: %s. Profile: %s. Device under test (DUT): %s on %s. Format: RFC 7502 §5 reporting with '
          'the RFC 6076 metrics.' % (spec.get('report_date') or datetime.date.today().isoformat(),
                                     'TEMPLATE (no evidence)' if template else spec.get('status', 'final'),
                                     spec.get('profile', PENDING), dut_label(spec), ctx['dut'].get('instance_type', PENDING)))
    if spec.get('summary'):
        doc.h(2, 'Summary')
        doc.ul(spec['summary'])
    number = iter(range(1, 100))
    section_headline(doc, ctx, '%d. Headline: Sustained Session Establishment Rate (>= 180 s trials)' % next(number))
    section_baseline(doc, ctx, '%d. Testbed baseline (RFC 7502 §6.1)' % next(number))
    section_setup(doc, ctx, '%d. Test setup report (RFC 7502 §5.1)' % next(number))
    section_benchmarks(doc, ctx, '%d. Device benchmarks for session setup (RFC 7502 §5.2)' % next(number))
    section_rfc6076(doc, ctx, '%d. RFC 6076 end-to-end metrics (final passing trial at R)' % next(number))
    section_latency_below(doc, ctx, '%d. Setup latency below the limit' % next(number))
    section_failures(doc, ctx, '%d. Session Attempt Failure breakdown (first failing rate above R)' % next(number))
    if spec.get('overload'):
        section_overload(doc, ctx, '%d. Overload and recovery (R4)' % next(number))
    section_traces(doc, ctx, '%d. Search traces (RFC 7502 §4.10)' % next(number))
    if spec.get('findings'):
        doc.h(2, '%d. Findings' % next(number))
        doc.ul(spec['findings'])
    section_methodology(doc, ctx, '%d. Setup and methodology' % next(number))
    section_deviations(doc, ctx, '%d. Deviations from RFC 7501, RFC 7502 and RFC 6076' % next(number))
    section_evidence(doc, ctx, '%d. Evidence' % next(number))
    return doc, title


TEMPLATE_SPEC = {
    'title': 'rvoip-bench-uas SIP benchmark (RFC 7501/7502, RFC 6076): template',
    'profile': 'minor',
    'dut': {'name': 'rvoip-bench-uas', 'instance_type': 'c8i.4xlarge'},
    'testbed': {'caller_hosts': 2, 'caller_type': 'c8in.24xlarge', 'sources_per_host': 2, 'connections_per_source': 8},
    'rows': [{'row': 'R1'}, {'row': 'R2'}, {'row': 'R5'}, {'row': 'R6'}],
}


def render(spec, out_dir, stem='rfc7502-report', builder=None, template=False):
    """Write <out_dir>/<stem>.md and .html; returns (md_path, html_path)."""
    doc, title = build(spec, template) if builder is None else builder(spec)
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    md, page = out / (stem + '.md'), out / (stem + '.html')
    md.write_text(doc.markdown())
    page.write_text(doc.html(title))
    return md, page


def main(argv):
    if len(argv) == 3 and argv[1] == '--template':
        md, page = render(TEMPLATE_SPEC, argv[2], 'rfc7502-report-template', template=True)
    elif len(argv) == 3:
        md, page = render(json.loads(Path(argv[1]).read_text()), argv[2])
    else:
        print(__doc__)
        return 2
    print(md)
    print(page)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
