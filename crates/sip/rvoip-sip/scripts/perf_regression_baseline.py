#!/usr/bin/env python3
"""Verify and package an immutable performance-regression baseline."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import re
import shutil
import sys
import tempfile
from typing import Any


SCHEMA = "rvoip-perf-regression-baseline-v1"
HEX_64 = re.compile(r"^[0-9a-f]{64}$")
BASELINE_ID = re.compile(r"^\d{8}T\d{6}Z$")


class BaselineError(RuntimeError):
    """Raised when a reviewed baseline is incomplete or has changed."""


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_manifest(path: pathlib.Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise BaselineError(f"cannot read baseline manifest: {error}") from error
    if not isinstance(value, dict):
        raise BaselineError("baseline manifest must be a JSON object")
    return value


def safe_relative(value: Any, label: str) -> pathlib.PurePosixPath:
    if not isinstance(value, str) or not value:
        raise BaselineError(f"{label} must be a non-empty relative path")
    path = pathlib.PurePosixPath(value)
    if path.is_absolute() or ".." in path.parts or "." in path.parts:
        raise BaselineError(f"{label} is not a safe relative path: {value!r}")
    return path


def listed_files(manifest: dict[str, Any]) -> list[dict[str, Any]]:
    files = manifest.get("files")
    if not isinstance(files, list) or not files:
        raise BaselineError("baseline manifest has no files")
    seen: set[str] = set()
    normalized: list[dict[str, Any]] = []
    for item in files:
        if not isinstance(item, dict):
            raise BaselineError("baseline file entry must be an object")
        relative = safe_relative(item.get("path"), "baseline file path").as_posix()
        if relative in seen:
            raise BaselineError(f"duplicate baseline file path: {relative}")
        seen.add(relative)
        size = item.get("bytes")
        digest = item.get("sha256")
        if not isinstance(size, int) or size < 0:
            raise BaselineError(f"invalid baseline byte count: {relative}")
        if not isinstance(digest, str) or HEX_64.fullmatch(digest) is None:
            raise BaselineError(f"invalid baseline SHA-256: {relative}")
        normalized.append({"path": relative, "bytes": size, "sha256": digest})
    return normalized


def validate_manifest_shape(manifest: dict[str, Any]) -> list[dict[str, Any]]:
    if manifest.get("schema") != SCHEMA:
        raise BaselineError("unsupported performance-regression baseline schema")
    baseline_id = manifest.get("baseline_id")
    if not isinstance(baseline_id, str) or BASELINE_ID.fullmatch(baseline_id) is None:
        raise BaselineError("baseline_id must be a UTC timestamp identifier")
    qualification = manifest.get("qualification")
    if (
        not isinstance(qualification, dict)
        or qualification.get("release_evidence") is not False
        or not isinstance(qualification.get("permitted_use"), str)
        or not qualification["permitted_use"]
    ):
        raise BaselineError(
            "baseline qualification must mark the historical input as non-release evidence"
        )
    source = manifest.get("source")
    if (
        not isinstance(source, dict)
        or not isinstance(source.get("git_revision"), str)
        or not source["git_revision"]
        or source.get("git_status") not in {"clean", "dirty"}
    ):
        raise BaselineError("baseline source identity is incomplete")
    files = listed_files(manifest)
    file_paths = {item["path"] for item in files}
    comparisons = manifest.get("comparison_paths")
    if not isinstance(comparisons, list) or not comparisons:
        raise BaselineError("baseline manifest has no required comparison paths")
    normalized_comparison_list = [
        safe_relative(path, "baseline comparison path").as_posix()
        for path in comparisons
    ]
    if len(set(normalized_comparison_list)) != len(normalized_comparison_list):
        raise BaselineError("baseline comparison paths must be unique")
    normalized_comparisons = set(normalized_comparison_list)
    missing = sorted(normalized_comparisons - file_paths)
    if missing:
        raise BaselineError(
            "baseline comparison paths are absent from the file inventory: "
            + ", ".join(missing)
        )
    return files


def verify_tree(
    manifest_path: pathlib.Path,
    result_root: pathlib.Path,
    *,
    require_exact_inventory: bool = True,
) -> dict[str, Any]:
    manifest_path = manifest_path.expanduser().resolve()
    result_root = result_root.expanduser().resolve()
    if not manifest_path.is_file() or manifest_path.is_symlink():
        raise BaselineError(f"baseline manifest is unavailable: {manifest_path}")
    if not result_root.is_dir() or result_root.is_symlink():
        raise BaselineError(f"baseline result root is unavailable: {result_root}")
    manifest = load_manifest(manifest_path)
    files = validate_manifest_shape(manifest)
    expected: set[str] = set()
    for item in files:
        relative = pathlib.PurePosixPath(item["path"])
        path = result_root.joinpath(*relative.parts)
        resolved = path.resolve()
        try:
            resolved.relative_to(result_root)
        except ValueError as error:
            raise BaselineError(f"baseline file escapes its root: {relative}") from error
        if not path.is_file() or path.is_symlink():
            raise BaselineError(f"baseline file is unavailable: {relative}")
        if path.stat().st_size != item["bytes"]:
            raise BaselineError(f"baseline byte count changed: {relative}")
        if sha256_file(path) != item["sha256"]:
            raise BaselineError(f"baseline SHA-256 changed: {relative}")
        expected.add(relative.as_posix())
    if require_exact_inventory:
        excluded: set[str] = set()
        try:
            excluded.add(manifest_path.relative_to(result_root).as_posix())
        except ValueError:
            pass
        actual = {
            path.relative_to(result_root).as_posix()
            for path in result_root.rglob("*")
            if path.is_file()
            and path.relative_to(result_root).as_posix() not in excluded
        }
        missing = sorted(expected - actual)
        added = sorted(actual - expected)
        if missing or added:
            details = []
            if missing:
                details.append("missing " + ", ".join(missing))
            if added:
                details.append("unlisted " + ", ".join(added))
            raise BaselineError("baseline inventory mismatch: " + "; ".join(details))
    return manifest


def package_baseline(
    source_root: pathlib.Path,
    manifest_path: pathlib.Path,
    artifact_dir: pathlib.Path,
) -> pathlib.Path:
    source_root = source_root.expanduser().resolve()
    manifest_path = manifest_path.expanduser().resolve()
    artifact_dir = artifact_dir.expanduser().resolve()
    manifest = verify_tree(manifest_path, source_root)
    artifact_dir.mkdir(parents=True, exist_ok=True)
    destination = artifact_dir / "perf-regression-baseline"
    if destination.exists():
        raise BaselineError(f"baseline package already exists: {destination}")
    temporary = pathlib.Path(
        tempfile.mkdtemp(prefix=".perf-regression-baseline-", dir=artifact_dir)
    )
    try:
        shutil.copy2(manifest_path, temporary / "manifest.json")
        result_root = temporary / "perf-results"
        for item in listed_files(manifest):
            relative = pathlib.PurePosixPath(item["path"])
            source = source_root.joinpath(*relative.parts)
            target = result_root.joinpath(*relative.parts)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
        verify_tree(temporary / "manifest.json", result_root)
        os.replace(temporary, destination)
    except BaseException:
        shutil.rmtree(temporary, ignore_errors=True)
        raise
    return destination


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)
    verify = commands.add_parser("verify", help="verify a source or packaged baseline")
    verify.add_argument("--manifest", required=True)
    verify.add_argument("--result-root", required=True)
    package = commands.add_parser("package", help="verify and package a baseline")
    package.add_argument("--manifest", required=True)
    package.add_argument("--source-root", required=True)
    package.add_argument("--artifact-dir", required=True)
    return root


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    try:
        if args.command == "verify":
            manifest = verify_tree(
                pathlib.Path(args.manifest), pathlib.Path(args.result_root)
            )
            print(
                "performance regression baseline verified: "
                f"{manifest['baseline_id']} ({len(manifest['files'])} files)"
            )
        else:
            destination = package_baseline(
                pathlib.Path(args.source_root),
                pathlib.Path(args.manifest),
                pathlib.Path(args.artifact_dir),
            )
            print(destination)
    except BaselineError as error:
        print(f"performance regression baseline: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
