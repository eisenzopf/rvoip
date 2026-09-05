#!/usr/bin/env python3
"""Validate and render the deployment-oriented rvoip facade bundles."""

from __future__ import annotations

import argparse
from pathlib import Path
import subprocess
import sys
import tomllib


ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "crates/rvoip/Cargo.toml"
DOCUMENT = ROOT / "docs/FEATURE_BUNDLES.md"
BUNDLE_PREFIX = "bundle-"


class BundleError(RuntimeError):
    """The public bundle contract is internally inconsistent."""


def load_contract() -> tuple[dict[str, list[str]], list[dict[str, object]]]:
    with MANIFEST.open("rb") as handle:
        manifest = tomllib.load(handle)
    features = manifest["features"]
    bundles = manifest["package"]["metadata"]["rvoip"]["feature-bundles"]
    return features, bundles


def feature_closure(name: str, features: dict[str, list[str]]) -> set[str]:
    closure: set[str] = set()
    pending = [name]
    while pending:
        current = pending.pop()
        if current in closure:
            continue
        closure.add(current)
        for member in features.get(current, []):
            if member in features:
                pending.append(member)
            else:
                closure.add(member)
    return closure


def validate(features: dict[str, list[str]], bundles: list[dict[str, object]]) -> None:
    if features.get("default") != ["sip"]:
        raise BundleError("the compatibility default must remain exactly ['sip']")

    metadata_names = [str(bundle["feature"]) for bundle in bundles]
    declared_names = [name for name in features if name.startswith(BUNDLE_PREFIX)]
    if metadata_names != declared_names:
        raise BundleError(
            "bundle metadata order/names differ from Cargo features: "
            f"metadata={metadata_names!r}, features={declared_names!r}"
        )
    if len(metadata_names) != len(set(metadata_names)):
        raise BundleError("bundle metadata contains a duplicate feature name")

    required_fields = {
        "feature",
        "name",
        "members",
        "codecs",
        "system_dependencies",
        "maturity",
        "description",
    }
    for bundle in bundles:
        missing = required_fields - bundle.keys()
        if missing:
            raise BundleError(f"{bundle.get('feature', '<unnamed>')} lacks {sorted(missing)}")
        name = str(bundle["feature"])
        members = bundle["members"]
        if not isinstance(members, list) or not all(isinstance(item, str) for item in members):
            raise BundleError(f"{name} members must be a string array")
        if features[name] != members:
            raise BundleError(
                f"{name} Cargo membership {features[name]!r} differs from metadata {members!r}"
            )
        nested = [member for member in members if member.startswith(BUNDLE_PREFIX)]
        if nested:
            raise BundleError(f"{name} must be expressed in stable leaf/meta features, not {nested}")
        unknown = [
            member
            for member in members
            if member not in features and not member.startswith("dep:") and "/" not in member
        ]
        if unknown:
            raise BundleError(f"{name} names unknown members {unknown}")

    pure = feature_closure("bundle-full-pure-rust", features)
    forbidden = {item for item in pure if item == "opus" or item.endswith("/opus")}
    if forbidden:
        raise BundleError(f"pure-Rust bundle reaches native Opus features: {sorted(forbidden)}")

    native = feature_closure("bundle-full-native", features)
    if "opus" not in native:
        raise BundleError("native full bundle does not reach the Opus feature")

    carrier = feature_closure("bundle-carrier-sip", features)
    for codec in ("g729", "amr-nb", "amr-wb"):
        if codec not in carrier:
            raise BundleError(f"carrier bundle lost mainline codec feature {codec}")


def render(bundles: list[dict[str, object]]) -> str:
    rows = []
    for bundle in bundles:
        members = ", ".join(f"`{item}`" for item in bundle["members"])
        rows.append(
            f"| `{bundle['feature']}` | {bundle['name']} | {members} | "
            f"{bundle['codecs']} | {bundle['system_dependencies']} | {bundle['maturity']} |"
        )
    table = "\n".join(rows)
    details = "\n\n".join(
        f"### `{bundle['feature']}` — {bundle['name']}\n\n{bundle['description']}"
        for bundle in bundles
    )
    return f"""# RVoIP facade feature bundles

RVoIP's Cargo features remain composable leaf features. The additive
`bundle-*` features are stable starting points for common deployment shapes;
they do not replace or rename any existing feature.

This document is generated from `crates/rvoip/Cargo.toml` by
`scripts/ci/check_facade_feature_bundles.py`. Edit the manifest metadata and
run the script with `--write`; CI rejects hand-edited or drifting output.

## Bundle matrix

| Cargo feature | Deployment shape | Direct members | Audio codecs | Extra system dependency | Maturity |
| --- | --- | --- | --- | --- | --- |
{table}

G.711 mu-law and A-law are the baseline SIP codecs and need no opt-in codec
feature. AMR-NB, AMR-WB, and G.729 are pure-Rust implementations. Opus is a
first-class codec, but its current backend links `libopus`, so bundles that
include it say so explicitly.

## Choosing a bundle

{details}

## Cargo examples

```toml
# Small provider-neutral SIP service.
rvoip = {{ version = "0.3.9", default-features = false, features = ["bundle-sip-endpoint"] }}

# Carrier-facing service with the pure-Rust telephony codec set.
rvoip = {{ version = "0.3.9", default-features = false, features = ["bundle-carrier-sip"] }}

# Browser-to-SIP application gateway; install libopus on the build host.
rvoip = {{ version = "0.3.9", default-features = false, features = ["bundle-browser-gateway"] }}
```

Advanced users may continue selecting leaf features directly. Start from
`default-features = false` when the dependency graph must contain only the
surfaces named by the application.
"""


def verify_resolved_graph() -> None:
    def tree(feature: str) -> str:
        completed = subprocess.run(
            [
                "cargo",
                "tree",
                "--locked",
                "-p",
                "rvoip",
                "--no-default-features",
                "--features",
                feature,
                "--prefix",
                "none",
            ],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )
        if completed.returncode:
            raise BundleError(completed.stderr.strip() or f"cargo tree failed for {feature}")
        return completed.stdout

    pure = tree("bundle-full-pure-rust")
    if any(line.startswith("opus v") for line in pure.splitlines()):
        raise BundleError("bundle-full-pure-rust resolves the native opus crate")
    native = tree("bundle-full-native")
    if not any(line.startswith("opus v") for line in native.splitlines()):
        raise BundleError("bundle-full-native does not resolve the native opus crate")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--write", action="store_true", help="regenerate the Markdown document")
    parser.add_argument(
        "--verify-resolved", action="store_true", help="also inspect Cargo's resolved graphs"
    )
    args = parser.parse_args()

    try:
        features, bundles = load_contract()
        validate(features, bundles)
        expected = render(bundles)
        if args.write:
            DOCUMENT.write_text(expected, encoding="utf-8")
        elif not DOCUMENT.is_file() or DOCUMENT.read_text(encoding="utf-8") != expected:
            raise BundleError(
                "docs/FEATURE_BUNDLES.md is stale; run "
                "python3 scripts/ci/check_facade_feature_bundles.py --write"
            )
        if args.verify_resolved:
            verify_resolved_graph()
    except (BundleError, KeyError, OSError, tomllib.TOMLDecodeError) as error:
        print(f"feature bundle check failed: {error}", file=sys.stderr)
        return 1
    print(f"validated {len(bundles)} facade feature bundles")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
