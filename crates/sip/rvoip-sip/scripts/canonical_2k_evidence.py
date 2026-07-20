#!/usr/bin/env python3
"""Validate and package three canonical rvoip-sip 2,000-CPS passes."""

import argparse
import datetime
import hashlib
import importlib.util
import json
import os
import pathlib
import shutil
import stat
import subprocess
import sys
import tempfile


SCRIPT_DIR = pathlib.Path(__file__).resolve().parent
ACCEPTANCE_SCRIPT = SCRIPT_DIR / "perf_2k_acceptance.py"
ACCEPTANCE_SPEC = importlib.util.spec_from_file_location(
    "canonical_evidence_perf_2k_acceptance", ACCEPTANCE_SCRIPT
)
ACCEPTANCE = importlib.util.module_from_spec(ACCEPTANCE_SPEC)
ACCEPTANCE_SPEC.loader.exec_module(ACCEPTANCE)

CANONICAL_SCENARIO = "perf_call_setup_cps_pbx-media-server"
MANIFEST_SCHEMA = "rvoip-perf-profile-manifest-v2"
INDEX_SCHEMA = "rvoip-canonical-2k-evidence-v2"
ACCEPTANCE_SCHEMA = "rvoip-sip-2k-acceptance-v3"


class EvidenceError(RuntimeError):
    pass


def git_bytes(root, *args):
    result = subprocess.run(
        ["git", *args],
        cwd=root,
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return result.stdout


def hash_frame(digest, value):
    digest.update(len(value).to_bytes(8, "little"))
    digest.update(value)


def capture_source_provenance(root):
    root = pathlib.Path(root).resolve()
    try:
        commit = git_bytes(root, "rev-parse", "HEAD").decode().strip()
        short = git_bytes(root, "rev-parse", "--short", "HEAD").decode().strip()
        status = git_bytes(
            root, "status", "--porcelain=v1", "-z", "--untracked-files=all"
        )
        tracked_diff = git_bytes(root, "diff", "--binary", "HEAD", "--", ".")
        untracked = sorted(
            path
            for path in git_bytes(
                root, "ls-files", "--others", "--exclude-standard", "-z"
            ).split(b"\0")
            if path
        )
        digest = hashlib.sha256(b"rvoip-source-fingerprint-v1\0")
        hash_frame(digest, commit.encode())
        hash_frame(digest, status)
        hash_frame(digest, tracked_diff)
        for raw_path in untracked:
            hash_frame(digest, raw_path)
            try:
                content = (root / os.fsdecode(raw_path)).read_bytes()
            except OSError as error:
                content = f"unreadable:{error.__class__.__name__}".encode()
            hash_frame(digest, content)
        return {
            "git_commit": commit,
            "git_rev": short,
            "git_dirty": bool(status),
            "source_fingerprint_sha256": digest.hexdigest(),
        }
    except (OSError, subprocess.CalledProcessError, UnicodeError) as error:
        return {
            "git_commit": "unknown",
            "git_rev": "unknown",
            "git_dirty": None,
            "source_fingerprint_sha256": "unknown",
            "error": str(error),
        }


def valid_fingerprint(value):
    return (
        isinstance(value, str)
        and len(value) == 64
        and all(character in "0123456789abcdef" for character in value)
    )


def load_json(path, label):
    try:
        value = json.loads(pathlib.Path(path).read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise EvidenceError(f"cannot read {label} {path}: {error}") from error
    if not isinstance(value, dict):
        raise EvidenceError(f"{label} must be a JSON object: {path}")
    return value


def require_equal(value, key, expected, label):
    actual = value.get(key)
    if actual != expected:
        raise EvidenceError(
            f"{label}.{key} must be {expected!r}, found {actual!r}"
        )


def tree_sha256(root):
    digest = hashlib.sha256(b"rvoip-canonical-evidence-tree-v1\0")
    for path in sorted(pathlib.Path(root).rglob("*")):
        if path.is_symlink():
            raise EvidenceError(f"evidence tree may not contain symlinks: {path}")
        if not path.is_file():
            continue
        relative = path.relative_to(root).as_posix().encode()
        hash_frame(digest, relative)
        hash_frame(digest, path.read_bytes())
    return digest.hexdigest()


def file_sha256(path):
    digest = hashlib.sha256()
    try:
        with pathlib.Path(path).open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(chunk)
    except OSError as error:
        raise EvidenceError(f"cannot hash executable {path}: {error}") from error
    return digest.hexdigest()


def parse_timestamp(value, label):
    if not isinstance(value, str):
        raise EvidenceError(f"{label} must be an ISO-8601 string")
    try:
        parsed = datetime.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise EvidenceError(f"invalid {label}: {value!r}") from error
    if parsed.tzinfo is None:
        raise EvidenceError(f"{label} must include a timezone: {value!r}")
    return parsed


def validate_run(run_dir, expected_fingerprint):
    run_dir = pathlib.Path(run_dir).resolve()
    if not run_dir.is_dir():
        raise EvidenceError(f"canonical run directory does not exist: {run_dir}")
    manifest = load_json(run_dir / "manifest.json", "manifest")
    for key, expected in (
        ("schema", MANIFEST_SCHEMA),
        ("mode", "clean"),
        ("scenario", CANONICAL_SCENARIO),
        ("status", 0),
        ("overall_status", "PASS"),
        ("run_executed", True),
        ("test_exit_code", 0),
        ("report_status", "CAPTURED"),
        ("acceptance_status", "PASS"),
        ("perf_audit_status", "PASS"),
        ("perf_audit_exit_code", 0),
        ("source_fingerprint_matches_runtime", True),
        ("source_fingerprint_matches_finalize", True),
        ("source_fingerprint_unchanged_for_full_run", True),
    ):
        require_equal(manifest, key, expected, "manifest")

    fingerprints = {
        "source_at_build": (manifest.get("source_at_build") or {}).get(
            "source_fingerprint_sha256"
        ),
        "runtime": (manifest.get("environment") or {}).get(
            "source_fingerprint_sha256"
        ),
        "source_at_finalize": (manifest.get("source_at_finalize") or {}).get(
            "source_fingerprint_sha256"
        ),
    }
    for label, fingerprint in fingerprints.items():
        if not valid_fingerprint(fingerprint):
            raise EvidenceError(
                f"{run_dir.name} has invalid or unknown {label} fingerprint"
            )
        if fingerprint != expected_fingerprint:
            raise EvidenceError(
                f"{run_dir.name} {label} fingerprint differs from beta source"
            )

    report_path = run_dir / "report.json"
    report = load_json(report_path, "report")
    require_equal(report, "scenario", CANONICAL_SCENARIO, "report")
    report_fingerprint = (report.get("environment") or {}).get(
        "source_fingerprint_sha256"
    )
    if report_fingerprint != expected_fingerprint:
        raise EvidenceError(f"{run_dir.name} report source fingerprint differs")
    reevaluated = ACCEPTANCE.evaluate(report, CANONICAL_SCENARIO, report_path)
    if reevaluated.get("status") != "PASS":
        failed = [
            check.get("metric")
            for check in reevaluated.get("checks", [])
            if not check.get("passed")
        ]
        raise EvidenceError(
            f"{run_dir.name} fails current canonical acceptance: {failed!r}"
        )

    acceptance = load_json(run_dir / "acceptance.json", "acceptance")
    require_equal(acceptance, "schema", ACCEPTANCE_SCHEMA, "acceptance")
    require_equal(acceptance, "status", "PASS", "acceptance")
    if not acceptance.get("checks") or any(
        check.get("passed") is not True for check in acceptance["checks"]
    ):
        raise EvidenceError(f"{run_dir.name} persisted acceptance has failed checks")

    audit_path = run_dir / "perf-audit.md"
    try:
        audit_lines = audit_path.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        raise EvidenceError(f"cannot read audit {audit_path}: {error}") from error
    # `perf_audit.py` has always serialized a successful Markdown verdict as
    # `status: OK`; the canonical manifest separately normalizes that result
    # to `perf_audit_status=PASS`. Validate the producer's wire vocabulary
    # here instead of accepting a fixture-only spelling that no real run can
    # generate.
    if "status: OK" not in audit_lines:
        raise EvidenceError(f"{run_dir.name} perf audit is not OK")

    executable_digest = manifest.get("executable_sha256")
    if not valid_fingerprint(executable_digest):
        raise EvidenceError(f"{run_dir.name} exact executable hash is invalid or unknown")
    executable_value = manifest.get("executable")
    if not isinstance(executable_value, str) or not executable_value:
        raise EvidenceError(f"{run_dir.name} exact executable path is missing")
    executable_path = pathlib.Path(executable_value).resolve()
    if not executable_path.is_file():
        raise EvidenceError(
            f"{run_dir.name} exact executable does not exist: {executable_path}"
        )
    actual_executable_digest = file_sha256(executable_path)
    if actual_executable_digest != executable_digest:
        raise EvidenceError(
            f"{run_dir.name} exact executable content differs from manifest hash"
        )

    captured_at = parse_timestamp(manifest.get("captured_at_utc"), "captured_at_utc")
    return {
        "run_dir": run_dir,
        "captured_at": captured_at,
        "captured_at_utc": manifest["captured_at_utc"],
        "source_fingerprint_sha256": expected_fingerprint,
        "executable_path": executable_path,
        "executable_sha256": executable_digest,
        "tree_sha256": tree_sha256(run_dir),
    }


def require_current_source(workspace_root, beta_start_path):
    beta_start = load_json(beta_start_path, "beta-start source")
    expected = beta_start.get("source_fingerprint_sha256")
    if not valid_fingerprint(expected):
        raise EvidenceError("beta-start source fingerprint is invalid or unknown")
    current = capture_source_provenance(workspace_root)
    actual = current.get("source_fingerprint_sha256")
    if not valid_fingerprint(actual):
        raise EvidenceError("current source fingerprint is invalid or unknown")
    if actual != expected:
        raise EvidenceError(
            "source tree changed after beta-start fingerprint capture "
            f"(expected {expected}, found {actual})"
        )
    return beta_start


def make_read_only(root):
    root = pathlib.Path(root)
    for path in sorted(root.rglob("*"), reverse=True):
        mode = stat.S_IMODE(path.stat().st_mode)
        if path.is_dir():
            path.chmod(mode & ~0o222 | 0o555)
        else:
            path.chmod(mode & ~0o222)
    root.chmod(stat.S_IMODE(root.stat().st_mode) & ~0o222 | 0o555)


def import_evidence(workspace_root, beta_start_path, artifact_dir, run_dirs):
    if len(run_dirs) != 3:
        raise EvidenceError(
            f"exactly three canonical run directories are required, found {len(run_dirs)}"
        )
    resolved = [pathlib.Path(path).resolve() for path in run_dirs]
    if len(set(resolved)) != 3:
        raise EvidenceError("canonical run directories must be distinct")

    beta_start = require_current_source(workspace_root, beta_start_path)
    fingerprint = beta_start["source_fingerprint_sha256"]
    validated = [validate_run(path, fingerprint) for path in resolved]
    executable_hashes = {item["executable_sha256"] for item in validated}
    if len(executable_hashes) != 1:
        raise EvidenceError(
            "canonical runs were not produced by one identical exact executable"
        )
    common_executable_sha256 = next(iter(executable_hashes))
    timestamps = [item["captured_at"] for item in validated]
    if any(left >= right for left, right in zip(timestamps, timestamps[1:])):
        raise EvidenceError(
            "canonical run directories must be supplied in strictly chronological order"
        )
    artifact_dir = pathlib.Path(artifact_dir).resolve()
    destination = artifact_dir / "canonical-2k"
    if destination.exists():
        raise EvidenceError(f"canonical evidence destination already exists: {destination}")
    artifact_dir.mkdir(parents=True, exist_ok=True)
    staging = pathlib.Path(
        tempfile.mkdtemp(prefix=".canonical-2k-staging-", dir=artifact_dir)
    )
    try:
        packaged_executable = staging / "executable" / validated[0]["executable_path"].name
        packaged_executable.parent.mkdir(parents=True)
        shutil.copy2(validated[0]["executable_path"], packaged_executable)
        if file_sha256(packaged_executable) != common_executable_sha256:
            raise EvidenceError("packaged exact executable hash changed during copy")
        runs_index = []
        for index, item in enumerate(validated, start=1):
            target = staging / f"run-{index}"
            shutil.copytree(item["run_dir"], target)
            packaged_tree_sha256 = tree_sha256(target)
            if packaged_tree_sha256 != item["tree_sha256"]:
                raise EvidenceError(
                    f"{item['run_dir'].name} changed while its evidence was copied"
                )
            runs_index.append(
                {
                    "sequence": index,
                    "source_run_dir": str(item["run_dir"]),
                    "packaged_run_dir": f"run-{index}",
                    "captured_at_utc": item["captured_at_utc"],
                    "source_fingerprint_sha256": fingerprint,
                    "executable_sha256": item["executable_sha256"],
                    "source_tree_sha256": item["tree_sha256"],
                    "packaged_tree_sha256": packaged_tree_sha256,
                }
            )
        index_value = {
            "schema": INDEX_SCHEMA,
            "status": "PASS",
            "scenario": CANONICAL_SCENARIO,
            "run_count": 3,
            "source_at_beta_start": beta_start,
            "common_source_fingerprint_sha256": fingerprint,
            "common_executable_sha256": common_executable_sha256,
            "packaged_executable": packaged_executable.relative_to(staging).as_posix(),
            "runs": runs_index,
        }
        (staging / "index.json").write_text(
            json.dumps(index_value, indent=2) + "\n", encoding="utf-8"
        )
        (staging / "README.md").write_text(
            "# Canonical 2,000-CPS Evidence\n\n"
            "Three chronological clean PASS runs, validated against the current "
            "absolute acceptance and relative-audit gates and one beta-start "
            "source fingerprint. All runs used one byte-identical executable, "
            "which is packaged and re-hashed here. See `index.json` and each "
            "`run-N/manifest.json`.\n",
            encoding="utf-8",
        )
        staging.replace(destination)
        make_read_only(destination)
    except Exception:
        shutil.rmtree(staging, ignore_errors=True)
        shutil.rmtree(destination, ignore_errors=True)
        raise
    print(
        f"canonical 2k evidence: PASS (3 runs, source {fingerprint}, {destination})"
    )
    return destination


def write_fingerprint(workspace_root, output):
    source = capture_source_provenance(workspace_root)
    if not valid_fingerprint(source.get("source_fingerprint_sha256")):
        raise EvidenceError(f"cannot capture source fingerprint: {source!r}")
    output = pathlib.Path(output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(source, indent=2) + "\n", encoding="utf-8")
    print(f"source fingerprint: {source['source_fingerprint_sha256']}")


def main():
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)

    fingerprint = subparsers.add_parser("fingerprint")
    fingerprint.add_argument("--workspace-root", required=True)
    fingerprint.add_argument("--out", required=True)

    importer = subparsers.add_parser("import")
    importer.add_argument("--workspace-root", required=True)
    importer.add_argument("--beta-start", required=True)
    importer.add_argument("--artifact-dir", required=True)
    importer.add_argument("--run-dir", action="append", default=[])

    verifier = subparsers.add_parser("verify-source")
    verifier.add_argument("--workspace-root", required=True)
    verifier.add_argument("--beta-start", required=True)

    args = parser.parse_args()
    try:
        if args.command == "fingerprint":
            write_fingerprint(args.workspace_root, args.out)
        elif args.command == "import":
            import_evidence(
                args.workspace_root,
                args.beta_start,
                args.artifact_dir,
                args.run_dir,
            )
        else:
            source = require_current_source(args.workspace_root, args.beta_start)
            print(
                "beta source unchanged: "
                f"{source['source_fingerprint_sha256']}"
            )
    except EvidenceError as error:
        print(f"canonical 2k evidence: FAIL: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
