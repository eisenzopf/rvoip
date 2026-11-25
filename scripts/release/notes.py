#!/usr/bin/env python3
"""Per-PR change fragments, assembled into the changelog and release notes.

Every pull request that changes behaviour adds one fragment,
`changes/<pr>.md` (or `changes/<pr>-<slug>.md`):

    ---
    section: Fixed
    breaking: false
    ---
    One Markdown paragraph (or a few) saying what changed for a user.

`check` validates every fragment (PR Gate). `assemble` (Prepare release PR)
appends the fragments to `## Unreleased` in CHANGELOG.md, renders them into
the generated block of docs/release/RELEASE_NOTES_NEXT.md, fills the
qualification record from the candidate's protected qualification evidence,
and deletes the fragments it used. Release preparation then moves
`## Unreleased` under the release heading as before.

The changelog, the notes and the fragments are release bookkeeping: no
qualification gate hashes them (scripts/release/gates.py), so finishing the
notes never invalidates qualified evidence.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import sys
from typing import Any


FRAGMENT_DIR = Path("changes")
CHANGELOG = Path("CHANGELOG.md")
NOTES = Path("docs/release/RELEASE_NOTES_NEXT.md")
FRAGMENT_NAME = re.compile(r"^(?P<pr>[1-9][0-9]*)(?:-[a-z0-9][a-z0-9-]*)?\.md$")
# CHANGELOG subsection order. "Highlights" stays hand-written.
SECTIONS = (
    "Breaking changes",
    "Added",
    "Changed",
    "Fixed",
    "Performance",
    "Security",
    "Deprecated",
    "Removed",
    "Documentation",
)
CHANGES_START = "<!-- generated:changes:start -->"
CHANGES_END = "<!-- generated:changes:end -->"
QUALIFICATION_START = "<!-- generated:qualification:start -->"
QUALIFICATION_END = "<!-- generated:qualification:end -->"
UNRELEASED = "## Unreleased"


class NotesError(RuntimeError):
    """A fragment or the notes failed validation."""


def parse_fragment(path: Path) -> dict[str, Any]:
    match = FRAGMENT_NAME.fullmatch(path.name)
    if not match:
        raise NotesError(
            f"{path}: name must be <pr-number>.md or <pr-number>-<slug>.md"
        )
    text = path.read_text(encoding="utf-8")
    front = re.match(r"\A---\n(?P<head>.*?)\n---\n(?P<body>.*)\Z", text, re.S)
    if not front:
        raise NotesError(f"{path}: must start with a --- front-matter block")
    fields: dict[str, str] = {}
    for line in front.group("head").splitlines():
        if not line.strip():
            continue
        key, separator, value = line.partition(":")
        if not separator:
            raise NotesError(f"{path}: front matter line {line!r} is not key: value")
        fields[key.strip()] = value.strip()
    unknown = sorted(set(fields) - {"section", "breaking"})
    if unknown:
        raise NotesError(f"{path}: unknown front-matter keys {unknown}")
    section = fields.get("section", "")
    if section not in SECTIONS:
        raise NotesError(f"{path}: section must be one of {list(SECTIONS)}, got {section!r}")
    breaking_value = fields.get("breaking", "false").lower()
    if breaking_value not in {"true", "false"}:
        raise NotesError(f"{path}: breaking must be true or false")
    breaking = breaking_value == "true" or section == "Breaking changes"
    body = front.group("body").strip()
    if not body:
        raise NotesError(f"{path}: the fragment says nothing")
    if body.lstrip().startswith("#"):
        raise NotesError(f"{path}: write prose or a list, not a heading")
    if breaking and "migrat" not in body.lower():
        raise NotesError(f"{path}: a breaking change must say how to migrate")
    return {
        "path": path,
        "pr": int(match.group("pr")),
        "section": section,
        "breaking": breaking,
        "body": body,
    }


def load_fragments(root: Path) -> list[dict[str, Any]]:
    directory = root / FRAGMENT_DIR
    if not directory.is_dir():
        return []
    fragments = [
        parse_fragment(path)
        for path in sorted(directory.iterdir())
        if path.is_file() and path.name != "README.md"
    ]
    return sorted(fragments, key=lambda item: (SECTIONS.index(item["section"]), item["pr"], item["path"].name))


def bullet(fragment: dict[str, Any]) -> str:
    lines = fragment["body"].splitlines()
    text = "\n".join(["- " + lines[0], *("  " + line if line else "" for line in lines[1:])])
    return f"{text} (#{fragment['pr']})"


def unreleased_bounds(text: str) -> tuple[int, int]:
    headings = list(re.finditer(r"(?m)^## [^\n]*$", text))
    for index, heading in enumerate(headings):
        if heading.group(0).strip() == UNRELEASED:
            end = headings[index + 1].start() if index + 1 < len(headings) else len(text)
            return heading.end(), end
    raise NotesError(f"{CHANGELOG} has no '{UNRELEASED}' section")


def changelog_with_fragments(text: str, fragments: list[dict[str, Any]]) -> str:
    start, end = unreleased_bounds(text)
    body = text[start:end].rstrip("\n")
    for section in SECTIONS:
        entries = [bullet(item) for item in fragments if item["section"] == section]
        if not entries:
            continue
        block = "\n\n".join(entries)
        heading = re.search(rf"(?m)^### {re.escape(section)}[ \t]*$", body)
        if heading:
            following = re.search(r"(?m)^### ", body[heading.end():])
            insert_at = heading.end() + following.start() if following else len(body)
            before = body[:insert_at].rstrip("\n")
            after = body[insert_at:]
            body = before + "\n\n" + block + ("\n\n" + after if after else "")
        else:
            body = body + f"\n\n### {section}\n\n{block}"
    return text[:start] + body + "\n\n" + text[end:].lstrip("\n")


def replace_block(text: str, start: str, end: str, content: str) -> str:
    if text.count(start) != 1 or text.count(end) != 1 or text.index(start) > text.index(end):
        raise NotesError(f"{NOTES} must contain exactly one {start} ... {end} block")
    before = text[: text.index(start) + len(start)]
    after = text[text.index(end) :]
    return before + "\n" + content.strip("\n") + "\n" + after


def render_changes(fragments: list[dict[str, Any]], existing: str) -> str:
    previous = existing.strip()
    lines = [] if not fragments else ["## Changes in this release", ""]
    breaking = [item for item in fragments if item["breaking"]]
    if breaking:
        lines += ["### Breaking changes summary", ""]
        lines += [bullet(item) for item in breaking]
        lines.append("")
    for section in SECTIONS:
        entries = [item for item in fragments if item["section"] == section and not item["breaking"]]
        if entries:
            lines += [f"### {section}", ""]
            lines += [bullet(item) for item in entries]
            lines.append("")
    rendered = "\n".join(lines).strip()
    if previous and rendered:
        return previous + "\n\n" + rendered
    return previous or rendered


def render_qualification(evidence: Path, run_url: str, version: str) -> str:
    aggregate_path = evidence / "release-qualification" / "aggregate.json"
    aggregate = json.loads(aggregate_path.read_text())
    if (
        aggregate.get("status") != "PASS"
        or aggregate.get("failures") != []
        or aggregate.get("profile") not in {"remote-release", "release-patch", "release-minor", "release-patch-smoke"}
    ):
        raise NotesError("the candidate's qualification aggregate is not a release-profile PASS")
    candidate = aggregate["candidate_sha"]
    if aggregate["profile"] == "release-patch-smoke":
        return (
            "## Qualification record\n\n"
            f"Candidate `{candidate}` passed `{aggregate['gate_count']}` gates in the protected "
            f"`release-patch-smoke` [workflow run]({run_url}): workspace crate tests, "
            "integration and regression tests, standalone examples, doc tests, direct ICE and TURN relay regressions, and package inventory checks. "
            "Benchmarks, load/soak tests, long fuzz campaigns, and external interoperability "
            "qualification were intentionally not run. This is reduced patch qualification.\n\n"
            f"The `{version}` version-bump commit must pass the same profile, bound to its exact "
            "commit, before protected publication. Reused receipts require matching inputs; "
            "the release archives include the accepted-gate ledger and explicit NOT RUN performance report."
        )
    jambonz = sorted((evidence / "release-evidence").glob("interop.jambonz-latest/latest.json"))
    jambonz_line = ""
    if jambonz:
        latest = json.loads(jambonz[0].read_text())
        jambonz_line = latest.get("selected_release_line", "")
    lines = [
        "## Qualification record",
        "",
        f"The release candidate `{candidate}` passed the protected "
        f"`{aggregate['profile']}` qualification ([workflow run]({run_url})): "
        f"{aggregate['gate_count']} of {aggregate['gate_count']} gates "
        f"({aggregate.get('fresh_count', 0)} run fresh, "
        f"{aggregate.get('reused_count', 0)} reused by exact inputs), including the "
        "complete crate matrix, feature bundles, CodeQL policy, Asterisk, "
        "FreeSWITCH, Jambonz OSS, Kamailio, OpenSIPS, SIPp, strict-UA, "
        "security, performance, resiliency, soak, regression and cleanup gates.",
        "",
    ]
    if jambonz_line:
        lines += [
            f"Jambonz OSS was tested at the pinned `{jambonz_line}` release line.",
            "",
        ]
    lines += [
        f"`{version}` is that candidate plus the version bump. Its requalification "
        "reuses these receipts through version-normalized input digests and "
        "reruns only the bookkeeping gates; the protected Release Publish "
        "workflow accepts only that requalification, bound to the exact "
        f"release commit, and the `v{version}` tag identifies it. The run's "
        "Performance evaluation report is attached to the GitHub release.",
    ]
    return "\n".join(lines)


def check(root: Path) -> int:
    fragments = load_fragments(root)
    notes = root / NOTES
    if notes.is_file():
        text = notes.read_text(encoding="utf-8")
        for start, end in ((CHANGES_START, CHANGES_END), (QUALIFICATION_START, QUALIFICATION_END)):
            replace_block(text, start, end, "")
    print(f"{len(fragments)} change fragment(s) valid")
    return 0


def assemble(root: Path, version: str, evidence: Path | None, run_url: str | None) -> int:
    fragments = load_fragments(root)
    notes_path = root / NOTES
    notes = notes_path.read_text(encoding="utf-8")
    existing = notes[notes.index(CHANGES_START) + len(CHANGES_START) : notes.index(CHANGES_END)] if CHANGES_START in notes else ""
    notes = replace_block(notes, CHANGES_START, CHANGES_END, render_changes(fragments, existing))
    if evidence is not None:
        if not run_url:
            raise NotesError("--qualification-evidence requires --run-url")
        notes = replace_block(
            notes,
            QUALIFICATION_START,
            QUALIFICATION_END,
            render_qualification(evidence, run_url, version),
        )
    changelog_path = root / CHANGELOG
    if fragments:
        changelog_path.write_text(
            changelog_with_fragments(changelog_path.read_text(encoding="utf-8"), fragments),
            encoding="utf-8",
        )
    notes_path.write_text(notes, encoding="utf-8")
    for fragment in fragments:
        fragment["path"].unlink()
    print(
        f"assembled {len(fragments)} change fragment(s) into {CHANGELOG} and {NOTES}"
        + ("; qualification record filled" if evidence is not None else "")
    )
    return 0


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    result.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    commands = result.add_subparsers(dest="command", required=True)
    commands.add_parser("check", help="validate every change fragment and the notes markers")
    build = commands.add_parser("assemble", help="fold fragments into the changelog and notes")
    build.add_argument("--version", required=True)
    build.add_argument(
        "--qualification-evidence",
        type=Path,
        help="downloaded release-gate-evidence of the candidate's qualification run",
    )
    build.add_argument("--run-url", help="URL of that qualification run")
    return result


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    try:
        if args.command == "check":
            return check(args.root)
        return assemble(args.root, args.version, args.qualification_evidence, args.run_url)
    except (NotesError, OSError, ValueError, KeyError) as error:
        print(f"release notes: FAIL: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
