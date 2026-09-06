#!/usr/bin/env python3
"""Fail a release until exact-main CodeQL is current and has no open alerts."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
from pathlib import Path
import time
from typing import Any
import urllib.error
import urllib.parse
import urllib.request


SCHEMA = "rvoip-codeql-release-policy-v1"
DEFAULT_CATEGORIES = (
    "/language:actions",
    "/language:c-cpp",
    "/language:javascript-typescript",
    "/language:python",
    "/language:rust",
)


class PolicyError(RuntimeError):
    """The candidate is not covered by a clean CodeQL analysis."""


def latest_codeql_by_category(analyses: list[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    latest: dict[str, dict[str, Any]] = {}
    for analysis in analyses:
        if analysis.get("tool", {}).get("name") != "CodeQL":
            continue
        category = analysis.get("category")
        if isinstance(category, str) and category not in latest:
            latest[category] = analysis
    return latest


def evaluate(
    analyses: list[dict[str, Any]],
    alerts: list[dict[str, Any]],
    candidate: str,
    required_categories: tuple[str, ...] = DEFAULT_CATEGORIES,
) -> dict[str, Any]:
    latest = latest_codeql_by_category(analyses)
    missing = [category for category in required_categories if category not in latest]
    stale = [
        category
        for category in required_categories
        if category in latest and latest[category].get("commit_sha") != candidate
    ]
    open_alerts = [alert for alert in alerts if alert.get("state") == "open"]
    failures: list[str] = []
    if missing:
        failures.append(f"missing CodeQL categories: {', '.join(missing)}")
    if stale:
        failures.append(f"CodeQL categories are not bound to {candidate}: {', '.join(stale)}")
    if open_alerts:
        failures.append(
            f"{len(open_alerts)} unreviewed CodeQL alert(s) remain open: "
            + ", ".join(str(alert.get("number", "unknown")) for alert in open_alerts[:20])
        )
    return {
        "schema": SCHEMA,
        "candidate": candidate,
        "required_categories": list(required_categories),
        "analysis_ids": {
            category: latest[category].get("id")
            for category in required_categories
            if category in latest
        },
        "open_alert_count": len(open_alerts),
        "open_alert_numbers": [alert.get("number") for alert in open_alerts],
        "status": "PASS" if not failures else "FAIL",
        "failures": failures,
    }


def api_pages(url: str, token: str) -> list[dict[str, Any]]:
    values: list[dict[str, Any]] = []
    while url:
        request = urllib.request.Request(
            url,
            headers={
                "Accept": "application/vnd.github+json",
                "Authorization": f"Bearer {token}",
                "User-Agent": "rvoip-release-policy",
                "X-GitHub-Api-Version": "2022-11-28",
            },
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                page = json.load(response)
                links = response.headers.get("Link", "")
        except (urllib.error.URLError, json.JSONDecodeError) as error:
            raise PolicyError(f"GitHub CodeQL API request failed: {type(error).__name__}") from error
        if not isinstance(page, list):
            raise PolicyError("GitHub CodeQL API returned a non-list response")
        values.extend(page)
        url = ""
        for part in links.split(","):
            if 'rel="next"' not in part:
                continue
            start = part.find("<")
            end = part.find(">", start + 1)
            if start >= 0 and end > start:
                url = part[start + 1 : end]
    return values


def fetch_state(api_url: str, repository: str, token: str) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    encoded_ref = urllib.parse.quote("refs/heads/main", safe="")
    base = f"{api_url.rstrip('/')}/repos/{repository}"
    analyses = api_pages(
        f"{base}/code-scanning/analyses?ref={encoded_ref}&per_page=100", token
    )
    alerts = api_pages(f"{base}/code-scanning/alerts?state=open&per_page=100", token)
    return analyses, alerts


def write_receipt(path: Path, result: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        **result,
        "checked_at": dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat(),
    }
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repository", required=True)
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--timeout-seconds", type=int, default=1_200)
    parser.add_argument("--poll-seconds", type=int, default=20)
    parser.add_argument("--api-url", default=os.environ.get("GITHUB_API_URL", "https://api.github.com"))
    args = parser.parse_args()

    token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    if not token:
        raise SystemExit("GH_TOKEN or GITHUB_TOKEN is required")

    deadline = time.monotonic() + args.timeout_seconds
    while True:
        analyses, alerts = fetch_state(args.api_url, args.repository, token)
        result = evaluate(analyses, alerts, args.candidate)
        write_receipt(args.receipt, result)
        analysis_is_stale = any(
            failure.startswith(("missing CodeQL categories", "CodeQL categories are not bound"))
            for failure in result["failures"]
        )
        if result["status"] == "PASS":
            print("exact-candidate CodeQL policy passed")
            return 0
        if not analysis_is_stale or time.monotonic() >= deadline:
            for failure in result["failures"]:
                print(f"- {failure}")
            return 1
        time.sleep(args.poll_seconds)


if __name__ == "__main__":
    raise SystemExit(main())
