#!/usr/bin/env python3
"""Create and verify the narrowly scoped rvoip 0.3.4 carry-forward release.

The inherited 0.3.2 beta exception remains immutable background evidence.  It
is never relabeled as a 0.3.4 beta run.  Current evidence is limited to one
clean canonical 2,000-CPS run bound to the exact release source fingerprint;
the unified release command separately requires current workspace tests and
doctests to pass before it writes a publishable verification receipt.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
from typing import Any


SCHEMA = "rvoip-release-carry-forward-attestation-v1"
RELEASE_VERSION = "0.3.4"
BASE_VERSION = "0.3.3"
BASE_TAG = "v0.3.3"
INHERITED_VERSION = "0.3.2"
INHERITED_DISPOSITION = "APPROVED-WITH-EXCEPTION"
INHERITED_STRICT_STATUS = "NON-RC"
INHERITED_RELATIVE_PATH = (
    "crates/sip/rvoip-sip/docs/releases/beta/20260729T010954Z/"
    "exception-r1/exception-attestation.json"
)
INHERITED_SHA256 = "fe9f6f6ec9b0d9db16d8b7d6d2f189819ca6d2f92ffe88a87911f6215cf649d7"
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
class CarryForwardError(RuntimeError):
    """A fail-closed carry-forward validation error."""


def repository_root() -> Path:
    return Path(__file__).resolve().parent.parent


def canonical_helper(root: Path) -> Any:
    path = root / "crates/sip/rvoip-sip/scripts/canonical_2k_evidence.py"
    spec = importlib.util.spec_from_file_location("rvoip_carry_forward_canonical", path)
    if not spec or not spec.loader:
        raise CarryForwardError(f"cannot load canonical evidence helper: {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(chunk)
    except OSError as error:
        raise CarryForwardError(f"cannot hash {path}: {error}") from error
    return digest.hexdigest()


def load_json(path: Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise CarryForwardError(f"cannot read {label} {path}: {error}") from error
    if not isinstance(value, dict):
        raise CarryForwardError(f"{label} must be a JSON object: {path}")
    return value


def require(condition: bool, message: str) -> None:
    if not condition:
        raise CarryForwardError(message)


def exact_keys(value: Any, expected: set[str], label: str) -> dict[str, Any]:
    require(isinstance(value, dict), f"{label} must be an object")
    actual = set(value)
    require(
        actual == expected,
        f"{label} fields differ "
        f"(missing={sorted(expected-actual)}, extra={sorted(actual-expected)})",
    )
    return value


def git_output(root: Path, *args: str) -> str:
    completed = subprocess.run(
        ["git", *args],
        cwd=root,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode:
        raise CarryForwardError(
            f"git {' '.join(args)} failed: {completed.stderr.strip()}"
        )
    return completed.stdout.strip()


def current_source(root: Path) -> dict[str, Any]:
    source = canonical_helper(root).capture_source_provenance(root)
    require(source.get("git_dirty") is False, "release source must be clean")
    require(
        COMMIT_RE.fullmatch(str(source.get("git_commit", ""))) is not None,
        "release commit is invalid",
    )
    require(
        COMMIT_RE.fullmatch(str(source.get("git_tree", ""))) is not None,
        "release tree is invalid",
    )
    require(
        SHA256_RE.fullmatch(str(source.get("source_fingerprint_sha256", "")))
        is not None,
        "release fingerprint is invalid",
    )
    return source


def validate_inherited_exception(root: Path) -> dict[str, Any]:
    path = root / INHERITED_RELATIVE_PATH
    require(path.is_file(), f"inherited 0.3.2 attestation is missing: {path}")
    require(sha256(path) == INHERITED_SHA256, "inherited 0.3.2 attestation hash changed")
    verifier = root / "scripts/release_exception_attestation.py"
    completed = subprocess.run(
        [
            sys.executable,
            str(verifier),
            "verify",
            "--attestation",
            str(path),
            "--version",
            INHERITED_VERSION,
        ],
        cwd=root,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
    )
    require(
        completed.returncode == 0,
        "inherited 0.3.2 attestation no longer verifies: "
        f"{completed.stdout.strip()}",
    )
    payload = load_json(path, "inherited attestation")
    release = payload.get("release")
    require(isinstance(release, dict), "inherited attestation lacks release metadata")
    require(release.get("version") == INHERITED_VERSION, "inherited release version changed")
    require(release.get("disposition") == INHERITED_DISPOSITION, "inherited disposition changed")
    require(release.get("strict_automated_status") == INHERITED_STRICT_STATUS, "inherited strict result changed")
    return {
        "version": INHERITED_VERSION,
        "disposition": INHERITED_DISPOSITION,
        "strict_automated_status": INHERITED_STRICT_STATUS,
        "attestation_path": INHERITED_RELATIVE_PATH,
        "attestation_sha256": INHERITED_SHA256,
    }


def release_delta(root: Path, head: str) -> dict[str, Any]:
    base = git_output(root, "rev-parse", "--verify", f"refs/tags/{BASE_TAG}^{{commit}}")
    require(COMMIT_RE.fullmatch(base) is not None, f"{BASE_TAG} does not resolve to a commit")
    ancestor = subprocess.run(
        ["git", "merge-base", "--is-ancestor", base, head],
        cwd=root,
        check=False,
    )
    require(ancestor.returncode == 0, f"{BASE_TAG} is not an ancestor of the release")
    paths = sorted(
        line
        for line in git_output(
            root,
            "diff",
            "--name-only",
            "--diff-filter=ACDMRTUXB",
            base,
            head,
        ).splitlines()
        if line
    )
    require(paths, "release delta from v0.3.3 is empty")
    return {
        "base_version": BASE_VERSION,
        "base_tag": BASE_TAG,
        "base_commit": base,
        "changed_path_count": len(paths),
        "changed_paths": paths,
    }


def canonical_facts(report: dict[str, Any]) -> dict[str, Any]:
    results = report.get("results") or {}
    latency = report.get("latency_ns") or {}
    resources = report.get("resources") or {}
    diagnostics = report.get("diagnostics") or {}
    errors = results.get("errors") or {}
    nonzero_errors = []
    stack: list[tuple[str, Any]] = [("results.errors", errors)]
    while stack:
        prefix, value = stack.pop()
        if isinstance(value, dict):
            stack.extend((f"{prefix}.{key}", child) for key, child in value.items())
        elif isinstance(value, (int, float)) and not isinstance(value, bool) and value != 0:
            nonzero_errors.append(prefix)
    settle = diagnostics.get("cleanup_convergence_at_settle") or {}
    final = diagnostics.get("cleanup_convergence") or {}
    require(
        report.get("scenario") == "perf_call_setup_cps_pbx-media-server",
        "canonical scenario changed",
    )
    require(
        results.get("calls_offered") == 65000
        and results.get("calls_succeeded") == 65000,
        "canonical call accounting failed",
    )
    require(
        float(results.get("achieved_cps", 0)) >= 1578.53,
        "canonical CPS is below the release threshold",
    )
    require(
        float(results.get("asr", 0)) >= 0.999
        and float(results.get("ner", 0)) >= 0.999,
        "canonical ASR/NER is below threshold",
    )
    require(not nonzero_errors, f"canonical errors are nonzero: {sorted(nonzero_errors)}")
    setup = latency.get("setup_latency") or {}
    require(
        int(setup.get("p99", 2**63 - 1)) <= 16_690_000,
        "canonical setup p99 exceeds threshold",
    )
    require(
        float(resources.get("peak_rss_mb", float("inf"))) <= 3202.26,
        "canonical peak RSS exceeds threshold",
    )
    require(
        float(resources.get("rss_active_growth_mb_per_min", float("inf")))
        <= 2378.44,
        "canonical active RSS growth exceeds threshold",
    )
    for label, convergence in (("settle", settle), ("final", final)):
        require(convergence.get("converged") is True, f"canonical {label} cleanup did not converge")
        require(
            convergence.get("retained_total") == 0,
            f"canonical {label} retained state is nonzero",
        )
        require(
            convergence.get("missing_count") == 0,
            f"canonical {label} cleanup metrics are missing",
        )
    return {
        "status": "PASS",
        "scenario": report["scenario"],
        "calls_offered": results["calls_offered"],
        "calls_succeeded": results["calls_succeeded"],
        "achieved_cps": results["achieved_cps"],
        "asr": results["asr"],
        "ner": results["ner"],
        "setup_p99_ns": setup["p99"],
        "peak_rss_mb": resources["peak_rss_mb"],
        "rss_active_growth_mb_per_min": resources["rss_active_growth_mb_per_min"],
        "settle_retained_total": settle["retained_total"],
        "final_retained_total": final["retained_total"],
    }


def validate_canonical_run(root: Path, run_dir: Path, source: dict[str, Any]) -> dict[str, Any]:
    helper = canonical_helper(root)
    try:
        validated = helper.validate_run(run_dir, source["source_fingerprint_sha256"])
    except Exception as error:
        raise CarryForwardError(f"canonical run validation failed: {error}") from error
    report = load_json(run_dir / "report.json", "canonical report")
    facts = canonical_facts(report)
    facts.update(
        {
            "captured_at_utc": validated["captured_at_utc"],
            "source_fingerprint_sha256": validated["source_fingerprint_sha256"],
            "executable_sha256": validated["executable_sha256"],
            "evidence_tree_sha256": validated["tree_sha256"],
        }
    )
    return facts


def artifact_manifest(root: Path) -> list[dict[str, Any]]:
    return [
        {
            "path": path.relative_to(root).as_posix(),
            "bytes": path.stat().st_size,
            "sha256": sha256(path),
        }
        for path in sorted(item for item in root.rglob("*") if item.is_file())
        if path.name
        not in {
            "carry-forward-attestation.json",
            "carry-forward-attestation.json.sha256",
        }
    ]


def write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def create(
    root: Path,
    run_dir: Path,
    output_dir: Path,
    version: str,
    approval_actor: str,
    approval_basis: str,
) -> Path:
    require(
        version == RELEASE_VERSION,
        f"carry-forward mode is limited to {RELEASE_VERSION}",
    )
    require(
        approval_actor.strip() != "" and approval_basis.strip() != "",
        "owner approval is required",
    )
    require(
        not output_dir.exists() or not any(output_dir.iterdir()),
        f"output directory must be absent or empty: {output_dir}",
    )
    source = current_source(root)
    inherited = validate_inherited_exception(root)
    delta = release_delta(root, source["git_commit"])
    facts = validate_canonical_run(root, run_dir, source)

    evidence_dir = output_dir / "evidence/canonical-2k"
    evidence_dir.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(run_dir, evidence_dir)
    copied_tree_sha256 = canonical_helper(root).tree_sha256(evidence_dir)
    require(
        copied_tree_sha256 == facts["evidence_tree_sha256"],
        "canonical evidence changed while it was copied",
    )
    inherited_dir = output_dir / "inherited"
    inherited_dir.mkdir(parents=True, exist_ok=True)
    shutil.copy2(root / INHERITED_RELATIVE_PATH, inherited_dir / "exception-attestation.json")

    payload = {
        "schema": SCHEMA,
        "created_at_utc": utc_now(),
        "release": {
            "version": version,
            "git_commit": source["git_commit"],
            "git_tree": source["git_tree"],
            "source_fingerprint_sha256": source["source_fingerprint_sha256"],
            "disposition": "OWNER-APPROVED-CARRY-FORWARD",
            "beta_suite": "NOT-RERUN",
            "current_workspace_verification": "REQUIRED-PASS",
        },
        "release_delta": delta,
        "inherited_beta_background": inherited,
        "current_evidence": {
            "canonical_2k": facts,
            "scope": "one clean canonical real-media run plus current workspace verification",
            "full_beta_suite": "NOT-RERUN",
            "interoperability_matrix": "NOT-RERUN",
            "long_soaks": "NOT-RERUN",
        },
        "approval": {
            "actor": approval_actor,
            "basis": approval_basis,
            "method": "explicit project-owner/operator instruction",
            "cryptographically_signed": False,
        },
        "artifacts": artifact_manifest(output_dir),
    }
    attestation = output_dir / "carry-forward-attestation.json"
    write_json(attestation, payload)
    (output_dir / "carry-forward-attestation.json.sha256").write_text(
        f"{sha256(attestation)}  carry-forward-attestation.json\n",
        encoding="utf-8",
    )
    return attestation


def verify(attestation: Path, version: str) -> dict[str, Any]:
    root = repository_root()
    require(
        version == RELEASE_VERSION,
        f"carry-forward mode is limited to {RELEASE_VERSION}",
    )
    require(attestation.is_file(), f"carry-forward attestation is missing: {attestation}")
    checksum = attestation.with_name("carry-forward-attestation.json.sha256")
    fields = (
        checksum.read_text(encoding="utf-8").strip().split()
        if checksum.is_file()
        else []
    )
    require(
        len(fields) == 2
        and fields[1] == attestation.name
        and fields[0] == sha256(attestation),
        "carry-forward attestation checksum mismatch",
    )
    payload = exact_keys(
        load_json(attestation, "carry-forward attestation"),
        {
            "schema",
            "created_at_utc",
            "release",
            "release_delta",
            "inherited_beta_background",
            "current_evidence",
            "approval",
            "artifacts",
        },
        "attestation",
    )
    require(payload["schema"] == SCHEMA, "unexpected carry-forward schema")
    source = current_source(root)
    release = exact_keys(
        payload["release"],
        {
            "version",
            "git_commit",
            "git_tree",
            "source_fingerprint_sha256",
            "disposition",
            "beta_suite",
            "current_workspace_verification",
        },
        "release",
    )
    expected_release = {
        "version": version,
        "git_commit": source["git_commit"],
        "git_tree": source["git_tree"],
        "source_fingerprint_sha256": source["source_fingerprint_sha256"],
        "disposition": "OWNER-APPROVED-CARRY-FORWARD",
        "beta_suite": "NOT-RERUN",
        "current_workspace_verification": "REQUIRED-PASS",
    }
    require(release == expected_release, "carry-forward release binding changed")
    require(
        payload["release_delta"] == release_delta(root, source["git_commit"]),
        "release delta changed",
    )
    require(
        payload["inherited_beta_background"] == validate_inherited_exception(root),
        "inherited beta background changed",
    )

    approval = exact_keys(
        payload["approval"],
        {"actor", "basis", "method", "cryptographically_signed"},
        "approval",
    )
    require(
        isinstance(approval["actor"], str) and approval["actor"].strip(),
        "approval actor is missing",
    )
    require(
        isinstance(approval["basis"], str) and approval["basis"].strip(),
        "approval basis is missing",
    )
    require(
        approval["method"] == "explicit project-owner/operator instruction"
        and approval["cryptographically_signed"] is False,
        "approval method changed",
    )

    evidence_root = attestation.parent
    artifacts = payload["artifacts"]
    require(isinstance(artifacts, list) and artifacts, "artifact manifest is missing")
    expected_paths: set[str] = set()
    for item in artifacts:
        record = exact_keys(item, {"path", "bytes", "sha256"}, "artifact")
        relative = record["path"]
        require(
            isinstance(relative, str)
            and relative not in expected_paths
            and not Path(relative).is_absolute()
            and ".." not in Path(relative).parts,
            f"invalid artifact path: {relative!r}",
        )
        expected_paths.add(relative)
        path = evidence_root / relative
        require(path.is_file(), f"attested artifact is missing: {relative}")
        require(
            path.stat().st_size == record["bytes"]
            and sha256(path) == record["sha256"],
            f"attested artifact changed: {relative}",
        )
    actual_paths = {
        path.relative_to(evidence_root).as_posix()
        for path in evidence_root.rglob("*")
        if path.is_file()
    } - {attestation.name, checksum.name}
    require(actual_paths == expected_paths, "carry-forward evidence contains unattested or missing files")
    require(
        sha256(evidence_root / "inherited/exception-attestation.json")
        == INHERITED_SHA256,
        "copied inherited attestation changed",
    )

    evidence = exact_keys(
        payload["current_evidence"],
        {"canonical_2k", "scope", "full_beta_suite", "interoperability_matrix", "long_soaks"},
        "current_evidence",
    )
    require(
        evidence["full_beta_suite"]
        == evidence["interoperability_matrix"]
        == evidence["long_soaks"]
        == "NOT-RERUN",
        "carry-forward evidence relabels an unrun beta gate",
    )
    report_path = evidence_root / "evidence/canonical-2k/report.json"
    report = load_json(report_path, "copied canonical report")
    helper = canonical_helper(root)
    reevaluated = helper.ACCEPTANCE.evaluate(report, helper.CANONICAL_SCENARIO, report_path)
    require(
        reevaluated.get("status") == "PASS",
        "copied canonical report fails current acceptance",
    )
    manifest = load_json(
        evidence_root / "evidence/canonical-2k/manifest.json",
        "copied canonical manifest",
    )
    require(
        manifest.get("overall_status") == "PASS"
        and manifest.get("acceptance_status") == "PASS"
        and manifest.get("perf_audit_status") == "PASS",
        "copied canonical manifest is not PASS",
    )
    require(
        (manifest.get("source_at_finalize") or {}).get(
            "source_fingerprint_sha256"
        )
        == source["source_fingerprint_sha256"],
        "canonical manifest fingerprint differs from release",
    )
    final_source = load_json(
        evidence_root / "evidence/canonical-2k/source-at-finalize.json",
        "copied final source",
    )
    require(
        final_source.get("git_commit") == source["git_commit"]
        and final_source.get("git_dirty") is False
        and final_source.get("source_fingerprint_sha256")
        == source["source_fingerprint_sha256"],
        "canonical run is not bound to the release source",
    )
    audit = (evidence_root / "evidence/canonical-2k/perf-audit.md").read_text(encoding="utf-8")
    require("status: OK" in audit, "canonical performance audit is not OK")
    expected_facts = canonical_facts(report)
    canonical = evidence["canonical_2k"]
    require(isinstance(canonical, dict), "canonical facts are missing")
    for key, value in expected_facts.items():
        require(canonical.get(key) == value, f"canonical fact changed: {key}")
    require(
        canonical.get("source_fingerprint_sha256")
        == source["source_fingerprint_sha256"],
        "canonical fact fingerprint changed",
    )
    require(
        canonical.get("executable_sha256") == manifest.get("executable_sha256")
        and SHA256_RE.fullmatch(str(canonical.get("executable_sha256", "")))
        is not None,
        "canonical executable hash is invalid or differs from its manifest",
    )
    copied_tree_sha256 = helper.tree_sha256(
        evidence_root / "evidence/canonical-2k"
    )
    require(
        canonical.get("evidence_tree_sha256") == copied_tree_sha256,
        "canonical evidence tree hash changed",
    )
    return payload


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(
        description="Create or verify the rvoip 0.3.4 carry-forward attestation."
    )
    commands = result.add_subparsers(dest="command", required=True)
    create_parser = commands.add_parser("create")
    create_parser.add_argument("--run-dir", type=Path, required=True)
    create_parser.add_argument("--output-dir", type=Path, required=True)
    create_parser.add_argument("--version", required=True)
    create_parser.add_argument("--approval-actor", required=True)
    create_parser.add_argument("--approval-basis", required=True)
    verify_parser = commands.add_parser("verify")
    verify_parser.add_argument("--attestation", type=Path, required=True)
    verify_parser.add_argument("--version", required=True)
    return result


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    root = repository_root()
    try:
        if args.command == "create":
            attestation = create(
                root,
                args.run_dir.resolve(),
                args.output_dir.resolve(),
                args.version,
                args.approval_actor,
                args.approval_basis,
            )
            print(f"carry-forward attestation created: {attestation}")
        else:
            verify(args.attestation.resolve(), args.version)
            print(f"carry-forward attestation: PASS ({args.version}, {args.attestation})")
    except CarryForwardError as error:
        print(f"carry-forward attestation: FAIL: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
