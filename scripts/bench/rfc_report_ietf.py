#!/usr/bin/env python3
"""IETF-format RFC 7501/7502/6076 report: Session Establishment Rate per row with the §6.1 testbed baseline.

Ported from the E74 harness (rfc_report_ietf.py); the minor profile renders
this edition. It reads the same SPEC.json as rfc_report.py and reuses its
evidence readers and section builders; only the arrangement differs (an
RFC 7501 terminology section, the baseline before the setup report, findings).

usage: rfc_report_ietf.py SPEC.json OUT_DIR      writes OUT_DIR/rfc7502-ietf-report.md and .html

Validity rule (kept from E74): a result is a true DUT limit only if R <= 0.67 x
the testbed baseline of the same row and generator; otherwise ">= R,
tester-limited". Missing evidence renders as "pending". Standard library only.
"""
import datetime
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import rfc_report as base  # noqa: E402
from rfc_report import PENDING, Doc  # noqa: E402


def build(spec):
    ctx = base.context(spec)
    doc = Doc()
    title = spec.get('title') or '%s: RFC 7501/7502/6076 SIP benchmark (IETF format)' % base.dut_label(spec)
    doc.h(1, title)
    if spec.get('label'):
        doc.p('Edition: %s.' % spec['label'])
    if spec.get('summary'):
        doc.h(2, 'Summary')
        doc.ul(spec['summary'])
    bed = ctx['testbed']
    doc.p('Report date %s. Device under test (DUT): %s on %s, a terminating SIP UAS and media endpoint. Tester: %s x %s '
          'Emulated Agents (UAC). Every number comes from valid trials run strictly serially (one DUT under load at a '
          'time, a fresh DUT process per trial).' % (
              spec.get('report_date') or datetime.date.today().isoformat(), base.dut_label(spec),
              ctx['dut'].get('instance_type', PENDING), base.fmt(bed.get('caller_hosts')), bed.get('caller_type', PENDING)))
    number = iter(range(1, 100))
    base.section_headline(doc, ctx, '%d. Headline: Sustained Session Establishment Rate (>= 180 s trials) per row' % next(number))
    base.section_terms(doc, '%d. RFC 7501 terminology used' % next(number))
    base.section_baseline(doc, ctx, '%d. Testbed baseline (RFC 7502 §6.1)' % next(number))
    base.section_setup(doc, ctx, '%d. Test setup report (RFC 7502 §5.1)' % next(number))
    base.section_benchmarks(doc, ctx, '%d. Device benchmarks (RFC 7502 §5.2)' % next(number))
    base.section_rfc6076(doc, ctx, '%d. RFC 6076 end-to-end metrics (final passing trial at R)' % next(number))
    base.section_latency_below(doc, ctx, '%d. Setup latency below the limit' % next(number))
    base.section_failures(doc, ctx, '%d. Session Attempt Failure breakdown (first failing rate above R)' % next(number))
    if spec.get('overload'):
        base.section_overload(doc, ctx, '%d. Overload and recovery (R4, not an RFC 7502 benchmark)' % next(number))
    doc.h(2, '%d. Findings' % next(number))
    doc.ul(spec.get('findings') or [PENDING])
    base.section_traces(doc, ctx, '%d. Search traces (RFC 7502 §4.10)' % next(number))
    base.section_methodology(doc, ctx, '%d. Setup and methodology' % next(number))
    base.section_deviations(doc, ctx, '%d. Deviations from RFC 7501, RFC 7502 and RFC 6076' % next(number))
    base.section_evidence(doc, ctx, '%d. Evidence' % next(number))
    return doc, title


def render(spec, out_dir, stem='rfc7502-ietf-report'):
    return base.render(spec, out_dir, stem, builder=build)


def main(argv):
    if len(argv) != 3:
        print(__doc__)
        return 2
    md, page = render(json.loads(Path(argv[1]).read_text()), argv[2])
    print(md)
    print(page)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
