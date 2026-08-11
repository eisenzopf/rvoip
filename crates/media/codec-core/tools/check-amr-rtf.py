#!/usr/bin/env python3
"""Turn the AMR Criterion run into a real-time-factor pass or fail.

Why this is a script and not a `cargo test`
-------------------------------------------
This repository deliberately keeps comparative timing out of `cargo test` --
see the header of `media-core/tests/g711_performance_benchmark.rs`. A debug
build under a loaded scheduler produces numbers that are meaningless as a
pass/fail signal, and a test that asserts on them is a flaky test that
everyone learns to rerun. Criterion samples properly, in release, and reports
a confidence interval; this reads what it wrote.

What the target means
---------------------
A 20 ms frame has a 20 ms budget. The real-time factor is `time / 20 ms`, so a
figure of 0.03 means one direction of one call costs 3% of one core.

The budget below is stated for a **duplex leg** -- encode plus decode, since a
call does both -- and the ceiling follows from the goal of 20 concurrent legs:
20 duplex legs at 0.05 would need exactly one core with nothing left for RTP,
jitter buffering or the rest of the stack, which is not a target so much as an
arithmetic coincidence. 0.04 leaves a fifth of the core, and the worst
measured pair (AMR-WB at 23.85) sits near 0.029.

Usage
-----
    cargo bench -p rvoip-codec-core --all-features --bench amr_codec
    python3 tools/check-amr-rtf.py

Reads `target/criterion/**/new/estimates.json`, which Criterion writes on
every run.
"""

from __future__ import annotations

import json
import pathlib
import sys

# One frame is 20 ms, in nanoseconds.
FRAME_NS = 20_000_000.0

# Ceilings on the real-time factor, per direction and for a duplex leg.
# Generous against the measured figures on purpose: this is a regression
# tripwire, not a target to tune against. A change that doubles the encoder
# trips it; ordinary machine-to-machine variation does not.
# Keyed by Criterion's own directory names: it replaces '/' in a group name
# with '_', so the `amr/encode` group lands in `amr_encode`.
LIMITS = {
    "amr_encode/amr-nb": 0.030,
    "amr_encode/amr-wb": 0.055,
    "amr_decode/amr-nb": 0.010,
    "amr_decode/amr-wb": 0.020,
    "amr_conceal/amr-nb": 0.010,
    "amr_conceal/amr-wb": 0.020,
}

# The duplex budget: the worst encode plus the worst decode of one variant.
DUPLEX_LIMIT = 0.040


def collect(root: pathlib.Path) -> dict[str, list[tuple[str, float]]]:
    """Group every AMR estimate by its `group/variant` prefix."""
    found: dict[str, list[tuple[str, float]]] = {}
    for estimates in root.glob("**/new/estimates.json"):
        # .../criterion/<group>/<variant>/<value>/new/estimates.json
        parts = estimates.relative_to(root).parts
        if len(parts) < 4 or not parts[0].startswith("amr_"):
            continue
        prefix = "/".join(parts[:2])
        label = "/".join(parts[2:-2])
        with estimates.open() as handle:
            data = json.load(handle)
        # The median is the figure to gate on: the mean is pulled by the
        # occasional scheduler stall, which is exactly the noise this is
        # supposed to ignore.
        nanos = data["median"]["point_estimate"]
        found.setdefault(prefix, []).append((label, nanos / FRAME_NS))
    return found


def main() -> int:
    here = pathlib.Path(__file__).resolve().parent
    # The workspace target directory, three levels up from this crate.
    root = here.parent.parent.parent.parent / "target" / "criterion"
    if not root.is_dir():
        print(f"no Criterion output at {root}", file=sys.stderr)
        print("run: cargo bench -p rvoip-codec-core --all-features --bench amr_codec", file=sys.stderr)
        return 2

    found = collect(root)
    if not found:
        print("Criterion output exists but holds no AMR results.", file=sys.stderr)
        print("This is a failure, not a pass: a gate that finds nothing to", file=sys.stderr)
        print("check must not report success.", file=sys.stderr)
        return 2

    failures = []
    worst = {}
    for prefix, entries in sorted(found.items()):
        limit = LIMITS.get(prefix)
        peak = max(entries, key=lambda pair: pair[1])
        worst[prefix] = peak[1]
        status = "     "
        if limit is None:
            status = "  ?  "
        elif peak[1] > limit:
            status = "FAIL "
            failures.append(f"{prefix} at {peak[0]}: RTF {peak[1]:.4f} > {limit:.4f}")
        print(f"{status}{prefix:<24} worst {peak[0]:<8} RTF {peak[1]:.4f}"
              + (f"  (limit {limit:.4f})" if limit else ""))

    for variant in ("amr-nb", "amr-wb"):
        encode = worst.get(f"amr_encode/{variant}")
        decode = worst.get(f"amr_decode/{variant}")
        if encode is None or decode is None:
            failures.append(f"{variant}: missing an encode or decode measurement")
            continue
        duplex = encode + decode
        ok = duplex <= DUPLEX_LIMIT
        print(f"{'     ' if ok else 'FAIL '}{variant} duplex leg          "
              f"RTF {duplex:.4f}  (limit {DUPLEX_LIMIT:.4f})")
        if not ok:
            failures.append(f"{variant} duplex: RTF {duplex:.4f} > {DUPLEX_LIMIT:.4f}")
        else:
            legs = int(1.0 / duplex)
            print(f"     {variant}: about {legs} duplex legs per core")

    if failures:
        print("\nreal-time factor regressed:", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1

    print("\nall AMR paths inside their real-time budget")
    return 0


if __name__ == "__main__":
    sys.exit(main())
