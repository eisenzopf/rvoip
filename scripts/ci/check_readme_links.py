#!/usr/bin/env python3
"""Validate release-facing README links without depending on the network."""

from __future__ import annotations

import argparse
from collections import defaultdict
import json
from pathlib import Path
import re
import sys
import tomllib
from urllib.parse import unquote, urlsplit


ROOT = Path(__file__).resolve().parents[2]
MARKDOWN_LINK = re.compile(
    r"!?\[[^\]]*\]\((?P<destination>[^)\s]+)(?:\s+[\"'][^\"']*[\"'])?\)"
)
MARKDOWN_BADGE_LINK = re.compile(
    r"\[!\[[^\]]*\]\([^)]+\)\]\((?P<destination>[^)\s]+)\)"
)
HTML_IMAGE = re.compile(r"<img\s+[^>]*src=[\"'](?P<destination>[^\"']+)[\"']", re.I)
HTML_LINK = re.compile(r"<a\s+[^>]*href=[\"'](?P<destination>[^\"']+)[\"']", re.I)
HEADING = re.compile(r"^#{1,6}\s+(.+?)\s*#*\s*$")
VERSION = re.compile(r"\b\d+\.\d+\.\d+\b")


def workspace_version(root: Path = ROOT) -> str:
    with (root / "Cargo.toml").open("rb") as handle:
        return tomllib.load(handle)["workspace"]["package"]["version"]


def published_readme_paths(root: Path = ROOT) -> tuple[Path, ...]:
    with (root / "Cargo.toml").open("rb") as handle:
        workspace = tomllib.load(handle)["workspace"]
    paths = {Path("README.md")}
    for member in workspace.get("members", []):
        manifest = root / member / "Cargo.toml"
        if not manifest.is_file():
            continue
        with manifest.open("rb") as handle:
            package = tomllib.load(handle).get("package", {})
        if package.get("publish") is False or package.get("publish") == []:
            continue
        configured = package.get("readme")
        if configured is False:
            continue
        readme = manifest.parent / (
            configured if isinstance(configured, str) else "README.md"
        )
        if readme.is_file():
            paths.add(readme.relative_to(root))
    return tuple(sorted(paths))


def markdown_destinations(text: str) -> list[tuple[int, str]]:
    matches = (
        list(MARKDOWN_LINK.finditer(text))
        + list(MARKDOWN_BADGE_LINK.finditer(text))
        + list(HTML_IMAGE.finditer(text))
        + list(HTML_LINK.finditer(text))
    )
    return sorted(
        ((text.count("\n", 0, match.start()) + 1, match.group("destination")) for match in matches),
        key=lambda item: item[0],
    )


def github_anchors(text: str) -> set[str]:
    """Return the heading identifiers GitHub generates for ordinary README headings."""

    counts: dict[str, int] = defaultdict(int)
    anchors: set[str] = set()
    for line in text.splitlines():
        match = HEADING.match(line)
        if match is None:
            continue
        title = re.sub(r"<[^>]+>", "", match.group(1)).strip().lower()
        base = re.sub(r"[^\w\- ]", "", title, flags=re.UNICODE).replace(" ", "-")
        base = re.sub(r"-+", "-", base)
        duplicate = counts[base]
        counts[base] += 1
        anchors.add(base if duplicate == 0 else f"{base}-{duplicate}")
    return anchors


def validate_local_links(root: Path = ROOT) -> list[str]:
    errors: list[str] = []
    for relative in published_readme_paths(root):
        document = root / relative
        text = document.read_text(encoding="utf-8")
        local_anchors = github_anchors(text)
        for line, raw_destination in markdown_destinations(text):
            destination = unquote(raw_destination)
            parsed = urlsplit(destination)
            if parsed.scheme or destination.startswith("//"):
                continue
            if not parsed.path:
                if parsed.fragment and parsed.fragment not in local_anchors:
                    errors.append(f"{relative}:{line}: missing anchor #{parsed.fragment}")
                continue
            target = (document.parent / parsed.path).resolve()
            try:
                target.relative_to(root.resolve())
            except ValueError:
                errors.append(f"{relative}:{line}: link escapes repository: {destination}")
                continue
            if not target.exists():
                errors.append(f"{relative}:{line}: missing path: {destination}")
                continue
            if parsed.fragment and target.is_file() and target.suffix.lower() == ".md":
                target_anchors = github_anchors(target.read_text(encoding="utf-8"))
                if parsed.fragment not in target_anchors:
                    errors.append(
                        f"{relative}:{line}: missing target anchor: {destination}"
                    )
    return errors


def validate_release_surfaces(root: Path = ROOT) -> list[str]:
    version = workspace_version(root)
    errors: list[str] = []
    release_readmes = (
        Path("README.md"),
        Path("crates/rvoip/README.md"),
        Path("crates/sip/rvoip-sip/README.md"),
    )
    texts = {
        relative: (root / relative).read_text(encoding="utf-8")
        for relative in release_readmes
        if (root / relative).is_file()
    }
    combined = "\n".join(texts.values())
    requirements = (
        f"https://crates.io/crates/rvoip/{version}",
        f"https://docs.rs/rvoip/{version}/rvoip/",
        f"https://docs.rs/rvoip-sip/{version}/rvoip_sip/",
        f"https://img.shields.io/docsrs/rvoip/{version}",
    )
    for requirement in requirements:
        if requirement not in combined:
            errors.append(f"release README surfaces do not contain {requirement}")
    for relative, text in texts.items():
        release_destinations = [
            destination
            for _, destination in markdown_destinations(text)
            if urlsplit(destination).hostname
            in {"crates.io", "docs.rs", "img.shields.io"}
            and "rvoip" in destination
        ]
        stale = sorted(
            {
                found
                for destination in release_destinations
                for found in VERSION.findall(destination)
                if found != version
            }
        )
        if stale:
            errors.append(f"{relative}: stale release versions: {', '.join(stale)}")
    root_readme = texts[Path("README.md")]
    for crate in ("rvoip", "rvoip-sip"):
        badge = f"https://img.shields.io/crates/v/{crate}.svg"
        matching = [
            destination
            for _, destination in markdown_destinations(root_readme)
            if destination.startswith(badge)
        ]
        if len(matching) != 1 or f"release={version}" not in matching[0]:
            errors.append(
                f"README.md: {crate} badge must carry release={version} to bust stale image proxies"
            )
    return errors


def validate_current_release_evidence(root: Path = ROOT) -> list[str]:
    """Keep canonical and archived qualification evidence on the workspace release."""

    version = workspace_version(root)
    docs = root / "crates/sip/rvoip-sip/docs"
    errors: list[str] = []

    release_report = docs / "BETA_RELEASE_REPORT.md"
    performance_report = docs / "BETA_PERFORMANCE_REPORT.md"
    release_checklist = docs / "BETA_RELEASE_CHECKLIST.md"
    if f"# RVoIP {version} Release Qualification Report" not in release_report.read_text(
        encoding="utf-8"
    ):
        errors.append(f"canonical release report is not for {version}")
    if f"Release: `{version}`" not in performance_report.read_text(encoding="utf-8"):
        errors.append(f"canonical performance report is not for {version}")
    if (
        f"Current published qualified runtime crate version: `{version}`."
        not in release_checklist.read_text(encoding="utf-8")
    ):
        errors.append(f"release checklist does not identify {version} as published")

    for filename in (
        "QUALIFICATION_SUMMARY.json",
        "QUALIFICATION_REPORT_ATTESTATION.json",
    ):
        data = json.loads((docs / filename).read_text(encoding="utf-8"))
        if data.get("release", {}).get("version") != version:
            errors.append(f"canonical {filename} is not for {version}")

    history_root = docs / "releases/qualification"
    archives = []
    for report in history_root.glob("*/BETA_RELEASE_REPORT.md"):
        if f"# RVoIP {version} Release Qualification Report" in report.read_text(
            encoding="utf-8"
        ):
            archives.append(report.parent)
    if len(archives) != 1:
        errors.append(
            f"expected exactly one archived qualification for {version}; found {len(archives)}"
        )
        return errors

    archive = archives[0]
    required_archive_files = (
        "BETA_GATE_REPORT.md",
        "BETA_PERFORMANCE_REPORT.md",
        "BETA_RELEASE_REPORT.md",
        "QUALIFICATION_REPORT_ATTESTATION.json",
        "QUALIFICATION_REPORT_ATTESTATION.json.sha256",
        "QUALIFICATION_SUMMARY.json",
        "current-performance-artifact-index.json",
        "current-performance-evaluation.json",
        "current-performance-evaluation.md",
    )
    for filename in required_archive_files:
        if not (archive / filename).is_file():
            errors.append(f"archived {version} evidence is missing {filename}")

    history = (history_root / "README.md").read_text(encoding="utf-8")
    if f"| `{version}` at `" not in history or archive.name not in history:
        errors.append(f"qualification history does not index archived {version} evidence")
    return errors


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=ROOT)
    args = parser.parse_args(argv)
    errors = (
        validate_local_links(args.root)
        + validate_release_surfaces(args.root)
        + validate_current_release_evidence(args.root)
    )
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print(
        f"README release surfaces and local links pass for {workspace_version(args.root)}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
