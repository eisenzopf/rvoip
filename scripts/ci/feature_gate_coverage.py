#!/usr/bin/env python3
"""Map every feature-gated Cargo target to the CI commands that build or run it.

The crate shards build default features. A target with `required-features`
is skipped there without a word, and a test file that opens with
`#![cfg(feature = "...")]` compiles to an empty binary that passes. Either
kind runs only if some gate turns its features on. `rvoip-webrtc`'s
`sip_webrtc_acceptance` sat in exactly that hole for two months and rotted.

This module models the cargo commands each CI surface runs -- the crate
shards and SIP lanes, every specialty gate Main Gate runs, the other
workflows' literal cargo steps, and the release catalog -- and reports, per
feature-gated target, which of them build it and which run it.
`test_feature_gate_coverage.py` fails when a target has no gate.

    python3 scripts/ci/feature_gate_coverage.py            # table
    python3 scripts/ci/feature_gate_coverage.py --json     # machine-readable
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass, field
import fnmatch
import functools
import json
from pathlib import Path
import re
import shlex
import subprocess
import sys
from typing import Any, Iterable

sys.path.insert(0, str(Path(__file__).resolve().parent))
import pr_plan  # noqa: E402
import run_checks  # noqa: E402


ROOT = Path(__file__).resolve().parents[2]
WORKSPACES = ("Cargo.toml", "examples/Cargo.toml")

# Targets a debug CI job must not run, and the release catalog runs instead.
# Keyed by package, then feature: a target needing that feature is accepted as
# covered when the release tooling names it. Anything else needs a CI gate.
RELEASE_ONLY_FEATURES = {
    "rvoip-sip": {
        # The perf harness is release-only by contract: `EnvironmentBlock::
        # capture` panics under `debug_assertions` rather than publish a
        # number nobody should cite. scripts/release/gates.json runs these
        # targets with `--release --features perf-tests`; the burst and
        # split-soak binaries are driven by scripts/release/prebuilt_performance.py.
        "perf-tests": "release-only perf harness (scripts/release)",
    },
}
RELEASE_REFERENCE_ROOTS = ("scripts/release", "crates/sip/rvoip-sip/scripts")

# `#![cfg(feature = ...)]` test files no gate turns on yet, each with the
# reason. Only cfg-gated files may be listed: a target with
# `required-features` must always have a gate. The policy test fails when an
# entry here gains coverage, so the list cannot go stale.
KNOWN_CFG_GAPS = {
    ("rvoip-sip", "invite_planner_retention_qualification"): (
        "needs `perf-tests` and is `#[ignore]`d as a >90 s release "
        "qualification, but no release gate runs it yet"
    ),
}

TARGET_FLAGS = {
    "--lib",
    "--bins",
    "--tests",
    "--examples",
    "--benches",
    "--all-targets",
    "--doc",
}
NAMED_TARGET_FLAGS = {"--bin": "bin", "--test": "test", "--example": "example", "--bench": "bench"}
VALUE_FLAGS = {
    "-p",
    "--package",
    "--features",
    "-F",
    "--manifest-path",
    "--target",
    "--target-dir",
    "--profile",
    "-j",
    "--jobs",
    "--exclude",
    "--color",
    "--message-format",
    "--fuzz-dir",
}


@dataclass(frozen=True)
class Package:
    name: str
    manifest: Path
    features: dict[str, list[str]]
    workspace: str

    @property
    def root(self) -> str:
        return self.manifest.parent.relative_to(ROOT).as_posix()


@dataclass(frozen=True)
class Target:
    package: str
    kind: str  # test | example | bench | bin
    name: str
    required: tuple[str, ...]
    cfg: str | None  # crate-level `#![cfg(...)]` predicate text, if any
    source: str


@dataclass
class Invocation:
    """One cargo command as a gate runs it."""

    gate: str
    tier: str  # ci | release
    subcommand: str
    argv: list[str]
    packages: list[str] = field(default_factory=list)
    workspace: bool = False
    manifest: Path | None = None
    features: list[str] = field(default_factory=list)
    all_features: bool = False
    no_default: bool = False
    flags: set[str] = field(default_factory=set)
    named: dict[str, list[str]] = field(default_factory=dict)
    no_run: bool = False


# --------------------------------------------------------------------------
# Inventory


def cargo_metadata(manifest: Path) -> dict[str, Any]:
    completed = subprocess.run(
        [
            "cargo",
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
            str(manifest),
        ],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    if completed.returncode:
        raise RuntimeError(completed.stderr.strip() or f"cargo metadata failed for {manifest}")
    return json.loads(completed.stdout)


CFG_ATTRIBUTE = re.compile(r"#!\[\s*cfg\s*\(")


def crate_cfg(source: Path) -> str | None:
    """The crate-level `#![cfg(...)]` predicates of a target root, joined by `all`."""

    try:
        text = source.read_text(errors="replace")
    except OSError:
        return None
    predicates = []
    for match in CFG_ATTRIBUTE.finditer(text):
        depth, index = 1, match.end()
        while index < len(text) and depth:
            depth += {"(": 1, ")": -1}.get(text[index], 0)
            index += 1
        predicates.append(" ".join(text[match.end() : index - 1].split()))
    if not predicates:
        return None
    return predicates[0] if len(predicates) == 1 else f"all({', '.join(predicates)})"


def inventory() -> tuple[dict[str, Package], list[Target]]:
    packages: dict[str, Package] = {}
    targets: list[Target] = []
    for workspace in WORKSPACES:
        metadata = cargo_metadata(ROOT / workspace)
        members = set(metadata["workspace_members"])
        for raw in metadata["packages"]:
            if raw["id"] not in members:
                continue
            package = Package(raw["name"], Path(raw["manifest_path"]), raw["features"], workspace)
            packages[package.name] = package
            for target in raw["targets"]:
                kinds = [kind for kind in target["kind"] if kind in {"test", "example", "bench", "bin"}]
                if not kinds:
                    continue
                source = Path(target["src_path"])
                required = tuple(target.get("required-features") or ())
                cfg = crate_cfg(source) if kinds[0] == "test" else None
                if not required and not (cfg and "feature" in cfg):
                    continue
                targets.append(
                    Target(
                        package=package.name,
                        kind=kinds[0],
                        name=target["name"],
                        required=required,
                        cfg=cfg,
                        source=source.relative_to(ROOT).as_posix(),
                    )
                )
    return packages, sorted(targets, key=lambda t: (t.package, t.kind, t.name))


# --------------------------------------------------------------------------
# `#![cfg]` predicates


TOKEN = re.compile(r'\s*(?:(?P<ident>[A-Za-z_][A-Za-z0-9_]*)|(?P<string>"[^"]*")|(?P<punct>[(),=]))')


def evaluate_cfg(predicate: str, enabled: set[str]) -> bool:
    """Evaluate a cfg predicate; anything but `feature` is assumed true."""

    tokens: list[str] = []
    position = 0
    while position < len(predicate):
        match = TOKEN.match(predicate, position)
        if not match or match.end() == position:
            if predicate[position:].strip():
                raise ValueError(f"cannot parse cfg predicate {predicate!r}")
            break
        tokens.append(match.group(match.lastgroup))
        position = match.end()

    def parse(index: int) -> tuple[bool | None, int]:
        name = tokens[index]
        index += 1
        if index < len(tokens) and tokens[index] == "=":
            value = tokens[index + 1].strip('"')
            return (value in enabled if name == "feature" else None), index + 2
        if index < len(tokens) and tokens[index] == "(":
            index += 1
            values: list[bool | None] = []
            while tokens[index] != ")":
                value, index = parse(index)
                values.append(value)
                if tokens[index] == ",":
                    index += 1
            index += 1
            known = [value for value in values if value is not None]
            if name == "all":
                return all(known), index
            if name == "any":
                return (any(known) if known else None), index
            if name == "not":
                return (None if values[0] is None else not values[0]), index
            return None, index
        return None, index

    value, index = parse(0)
    if index != len(tokens):
        raise ValueError(f"trailing tokens in cfg predicate {predicate!r}")
    return value is not False


def cfg_features(predicate: str) -> list[str]:
    return sorted(set(re.findall(r'feature\s*=\s*"([^"]+)"', predicate)))


# --------------------------------------------------------------------------
# Commands


def parse_invocation(
    argv: list[str], *, gate: str, tier: str, cwd: Path | None = None
) -> Invocation | None:
    """Model a `cargo test|build|check|clippy` argv; None for anything else."""

    tokens = list(argv)
    if "cargo" not in tokens:
        return None
    tokens = tokens[tokens.index("cargo") + 1 :]
    while tokens and tokens[0].startswith("+"):
        tokens.pop(0)
    if not tokens or tokens[0] not in {"test", "build", "check", "clippy"}:
        return None
    invocation = Invocation(gate=gate, tier=tier, subcommand=tokens[0], argv=list(argv))
    if cwd is not None and (cwd / "Cargo.toml").is_file():
        invocation.manifest = (cwd / "Cargo.toml").resolve()
    index = 1
    while index < len(tokens):
        token = tokens[index]
        if token == "--":
            break
        value = None
        if "=" in token and token.split("=", 1)[0] in VALUE_FLAGS | set(NAMED_TARGET_FLAGS):
            token, value = token.split("=", 1)
        elif token in VALUE_FLAGS or token in NAMED_TARGET_FLAGS:
            index += 1
            value = tokens[index] if index < len(tokens) else ""
        if token in {"-p", "--package"}:
            invocation.packages.append(value)
        elif token in {"--features", "-F"}:
            invocation.features.extend(
                item for item in re.split(r"[,\s]+", value or "") if item
            )
        elif token == "--manifest-path":
            path = Path(value.replace("{workspace}", str(ROOT)))
            invocation.manifest = (path if path.is_absolute() else ROOT / path).resolve()
        elif token == "--workspace":
            invocation.workspace = True
        elif token == "--all-features":
            invocation.all_features = True
        elif token == "--no-default-features":
            invocation.no_default = True
        elif token == "--no-run":
            invocation.no_run = True
        elif token in TARGET_FLAGS:
            invocation.flags.add(token)
        elif token in NAMED_TARGET_FLAGS:
            invocation.named.setdefault(NAMED_TARGET_FLAGS[token], []).append(value)
        index += 1
    return invocation


def selected_packages(invocation: Invocation, packages: dict[str, Package]) -> set[str]:
    if invocation.packages:
        return {name for name in invocation.packages if name in packages}
    manifest = invocation.manifest or (ROOT / "Cargo.toml").resolve()
    for package in packages.values():
        if package.manifest.resolve() == manifest:
            return {package.name}
    workspace = manifest.relative_to(ROOT.resolve()).as_posix()
    return {package.name for package in packages.values() if package.workspace == workspace}


def enabled_features(invocation: Invocation, package: Package, selected: set[str]) -> set[str]:
    if invocation.all_features:
        return set(package.features)
    enabled = set() if invocation.no_default else {"default"}
    for feature in invocation.features:
        owner, _, name = feature.rpartition("/")
        if owner:
            if owner == package.name:
                enabled.add(name)
        elif len(selected) == 1 or name in package.features:
            enabled.add(name)
    pending = list(enabled)
    while pending:
        for value in package.features.get(pending.pop(), []):
            if value.startswith("dep:") or "/" in value or value in enabled:
                continue
            enabled.add(value)
            pending.append(value)
    return enabled


def selects(invocation: Invocation, target: Target) -> bool:
    flags, named = invocation.flags, invocation.named
    if any(fnmatch.fnmatchcase(target.name, pattern) for pattern in named.get(target.kind, [])):
        return True
    if "--all-targets" in flags:
        return True
    plural = {"test": "--tests", "example": "--examples", "bench": "--benches", "bin": "--bins"}
    if plural[target.kind] in flags:
        return True
    if flags or named:
        return False
    # No target selection: `cargo test` builds lib, bins, tests and examples;
    # build/check/clippy build lib and bins.
    if invocation.subcommand == "test":
        return target.kind in {"test", "example", "bin"}
    return target.kind == "bin"


def runs(invocation: Invocation, target: Target) -> bool:
    return invocation.subcommand == "test" and not invocation.no_run and target.kind == "test"


def covers(
    invocation: Invocation, target: Target, packages: dict[str, Package]
) -> str | None:
    """'run', 'build' or None for whether `invocation` reaches `target`."""

    selected = selected_packages(invocation, packages)
    if target.package not in selected or not selects(invocation, target):
        return None
    enabled = enabled_features(invocation, packages[target.package], selected)
    if not set(target.required) <= enabled:
        return None
    if target.cfg and not evaluate_cfg(target.cfg, enabled):
        return None
    return "run" if runs(invocation, target) else "build"


# --------------------------------------------------------------------------
# Gate surfaces


def main_specialty_gates() -> list[str]:
    """Every specialty gate Main Gate runs on each push to main."""

    workflow = (ROOT / ".github/workflows/main-ci.yml").read_text()
    forced = set(re.findall(r"--specialty-gate ([A-Za-z0-9_-]+)", workflow))
    # The workflow adds one `example--<project>` gate per examples/ project.
    forced.update(
        f"example--{manifest.parent.name}"
        for manifest in sorted((ROOT / "examples").glob("*/Cargo.toml"))
    )
    # Main plans with `--changed-file Cargo.lock`, so every rule naming the
    # lockfile is selected as well.
    policy = json.loads((ROOT / "scripts/ci/policy.json").read_text())
    forced.update(
        rule["gate"]
        for rule in policy["specialty_rules"]
        if "Cargo.lock" in rule["patterns"]
    )
    return pr_plan.expand_specialty_parts(forced)


def workflow_commands(path: Path) -> list[str]:
    """The shell commands in a workflow's `run:` steps, continuations joined."""

    lines = path.read_text().splitlines()
    commands: list[str] = []
    index = 0
    run = re.compile(r"^(\s*)(?:-\s+)?run:\s*(.*)$")
    while index < len(lines):
        match = run.match(lines[index])
        index += 1
        if not match:
            continue
        indent, rest = len(match.group(1)), match.group(2).strip()
        if rest and rest[0] not in "|>":
            commands.append(rest)
            continue
        block: list[str] = []
        while index < len(lines) and (
            not lines[index].strip() or len(lines[index]) - len(lines[index].lstrip()) > indent
        ):
            block.append(lines[index].strip())
            index += 1
        if rest.startswith(">"):
            commands.append(" ".join(line for line in block if line))
        else:
            commands.extend(re.sub(r"\\\n", " ", "\n".join(block)).splitlines())
    return commands


def ci_invocations(packages: dict[str, Package]) -> list[Invocation]:
    invocations: list[Invocation] = []

    def add(argv: list[str], gate: str, cwd: Path | None = None) -> None:
        invocation = parse_invocation(argv, gate=gate, tier="ci", cwd=cwd)
        if invocation:
            invocations.append(invocation)

    root_members = sorted(name for name, p in packages.items() if p.workspace == "Cargo.toml")
    for name in root_members:
        if name == "rvoip-sip":
            continue  # Main Gate runs rvoip-sip in the SIP lanes, not a shard.
        for argv, cwd, _ in run_checks.shard_commands(name):
            add(argv, f"shard ({name})", cwd)
    metadata = cargo_metadata(ROOT / "Cargo.toml")
    sip_targets = pr_plan.integration_test_targets(metadata, "rvoip-sip")
    for argv, cwd, _ in (
        run_checks.sip_core_commands()
        + run_checks.sip_clippy_commands()
        + run_checks.sip_integration_commands(",".join(sip_targets))
    ):
        add(argv, "sip lanes", cwd)
    for gate in main_specialty_gates():
        for argv, cwd, _ in run_checks.specialty_commands(gate, ROOT):
            add(argv, gate, cwd)
    for workflow in sorted((ROOT / ".github/workflows").glob("*.yml")):
        if workflow.name.startswith("release-"):
            continue  # Release workflows run the catalog, modelled below.
        for command in workflow_commands(workflow):
            if "cargo" not in command or "run_checks.py" in command:
                continue
            try:
                argv = shlex.split(command.split("&&")[0].split("|")[0])
            except ValueError:
                continue
            add(argv, f"workflow {workflow.name}")
    return invocations


def release_invocations() -> list[Invocation]:
    catalog = json.loads((ROOT / "scripts/release/gates.json").read_text())
    invocations = []
    for gate in catalog["gates"]:
        argv = gate.get("command")
        if not isinstance(argv, list):
            continue
        invocation = parse_invocation(
            [str(item) for item in argv], gate=f"release {gate['id']}", tier="release"
        )
        if invocation:
            invocations.append(invocation)
    return invocations


@functools.lru_cache(maxsize=1)
def release_corpus() -> str:
    return "\n".join(
        path.read_text(errors="replace")
        for directory in RELEASE_REFERENCE_ROOTS
        for path in sorted((ROOT / directory).rglob("*"))
        if path.is_file()
        and path.suffix in {".json", ".py", ".sh", ".yaml", ".yml"}
        and not path.name.startswith("test_")
    )


def release_references(name: str) -> bool:
    """Whether the release tooling names `name` (e.g. a prebuilt perf binary)."""

    return re.search(rf"\b{re.escape(name)}\b", release_corpus()) is not None


# --------------------------------------------------------------------------
# Report


def needed_features(target: Target) -> list[str]:
    return sorted(set(target.required) | set(cfg_features(target.cfg or "")))


def release_only_reason(target: Target) -> str | None:
    for feature, reason in RELEASE_ONLY_FEATURES.get(target.package, {}).items():
        if feature in needed_features(target):
            return reason
    return None


def coverage() -> list[dict[str, Any]]:
    packages, targets = inventory()
    ci = ci_invocations(packages)
    release = release_invocations()
    rows = []
    for target in targets:
        ci_hits: dict[str, set[str]] = {"run": set(), "build": set()}
        for invocation in ci:
            mode = covers(invocation, target, packages)
            if mode:
                ci_hits[mode].add(invocation.gate)
        release_hits: dict[str, set[str]] = {"run": set(), "build": set()}
        for invocation in release:
            mode = covers(invocation, target, packages)
            if mode:
                release_hits[mode].add(invocation.gate)
        need = "run" if target.kind == "test" else "build"
        ci_ok = bool(ci_hits["run"]) if need == "run" else bool(ci_hits["run"] | ci_hits["build"])
        reason = release_only_reason(target)
        release_ok = bool(
            reason
            and (
                release_hits["run"]
                or (need == "build" and release_hits["build"])
                or release_references(target.name)
            )
        )
        status = "ci" if ci_ok else "release-only" if release_ok else "UNGATED"
        rows.append(
            {
                "package": target.package,
                "kind": target.kind,
                "target": target.name,
                "source": target.source,
                "gating": "required-features" if target.required else "cfg",
                "required_features": list(target.required),
                "cfg": target.cfg,
                "needs": need,
                "ci_run": sorted(ci_hits["run"]),
                "ci_build": sorted(ci_hits["build"] - ci_hits["run"]),
                "release_run": sorted(release_hits["run"]),
                "release_build": sorted(release_hits["build"] - release_hits["run"]),
                "release_only_reason": reason if status == "release-only" else None,
                "status": status,
            }
        )
    return rows


def ungated(rows: Iterable[dict[str, Any]]) -> list[dict[str, Any]]:
    return [
        row
        for row in rows
        if row["status"] == "UNGATED"
        and (row["gating"] == "required-features" or (row["package"], row["target"]) not in KNOWN_CFG_GAPS)
    ]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    rows = coverage()
    if args.json:
        print(json.dumps(rows, indent=2))
    else:
        for row in rows:
            features = ",".join(row["required_features"]) or f"cfg {row['cfg']}"
            gates = row["ci_run"] or row["ci_build"] or row["release_run"] or row["release_build"]
            print(
                f"{row['status']:<12} {row['package']:<22} {row['kind']:<7} "
                f"{row['target']:<38} {features:<45} {', '.join(gates) or '-'}"
            )
    return 1 if ungated(rows) else 0


if __name__ == "__main__":
    raise SystemExit(main())
