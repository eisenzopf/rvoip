"""rvoip release-worker janitor.

Invoked on a fixed EventBridge schedule. Terminates every EC2 instance that
carries ``managed-by=github-actions`` and whose ``rvoip-expires-at`` tag
(ISO 8601, UTC) is in the past. Controllers set the tag to creation + 4h, so
this only ever catches workers an interrupted controller left behind.

The function is deliberately conservative: an instance with a missing or
unparseable expiry tag is logged and left alone, and one failed terminate
does not stop the sweep.
"""

import json
import logging
import os
from datetime import datetime, timezone

import boto3
from botocore.exceptions import ClientError

LOG = logging.getLogger()
LOG.setLevel(logging.INFO)

MANAGED_BY_TAG_KEY = os.environ.get("MANAGED_BY_TAG_KEY", "managed-by")
MANAGED_BY_TAG_VALUE = os.environ.get("MANAGED_BY_TAG_VALUE", "github-actions")
EXPIRES_AT_TAG_KEY = os.environ.get("EXPIRES_AT_TAG_KEY", "rvoip-expires-at")

# States in which an instance still holds capacity or storage.
LIVE_STATES = ["pending", "running", "stopping", "stopped"]

_ec2 = boto3.client("ec2")


def _parse_expiry(value):
    """Parse an ISO 8601 timestamp; naive values are taken as UTC."""
    text = value.strip()
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    parsed = datetime.fromisoformat(text)
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def _managed_instances():
    paginator = _ec2.get_paginator("describe_instances")
    pages = paginator.paginate(
        Filters=[
            {"Name": f"tag:{MANAGED_BY_TAG_KEY}", "Values": [MANAGED_BY_TAG_VALUE]},
            {"Name": "instance-state-name", "Values": LIVE_STATES},
        ]
    )
    for page in pages:
        for reservation in page.get("Reservations", []):
            for instance in reservation.get("Instances", []):
                yield instance


def handler(event, context):
    now = datetime.now(timezone.utc)
    summary = {"checked": 0, "expired": [], "terminated": [], "skipped": [], "failed": []}

    for instance in _managed_instances():
        instance_id = instance["InstanceId"]
        state = instance.get("State", {}).get("Name")
        tags = {t["Key"]: t["Value"] for t in instance.get("Tags", [])}
        name = tags.get("Name", "")
        summary["checked"] += 1

        raw_expiry = tags.get(EXPIRES_AT_TAG_KEY)
        if not raw_expiry:
            LOG.warning("skip %s (%s, %s): no %s tag", instance_id, name, state, EXPIRES_AT_TAG_KEY)
            summary["skipped"].append(instance_id)
            continue

        try:
            expires_at = _parse_expiry(raw_expiry)
        except ValueError:
            LOG.warning(
                "skip %s (%s, %s): unparseable %s=%r", instance_id, name, state, EXPIRES_AT_TAG_KEY, raw_expiry
            )
            summary["skipped"].append(instance_id)
            continue

        if expires_at > now:
            LOG.info("keep %s (%s, %s): expires %s", instance_id, name, state, expires_at.isoformat())
            continue

        summary["expired"].append(instance_id)
        LOG.warning(
            "terminate %s (%s, %s): expired %s, run-id=%s shard=%s",
            instance_id,
            name,
            state,
            expires_at.isoformat(),
            tags.get("rvoip-run-id", "?"),
            tags.get("rvoip-shard-id", "?"),
        )
        try:
            _ec2.terminate_instances(InstanceIds=[instance_id])
            summary["terminated"].append(instance_id)
        except ClientError as exc:
            LOG.error("failed to terminate %s: %s", instance_id, exc)
            summary["failed"].append(instance_id)

    LOG.info("janitor summary: %s", json.dumps(summary))
    return summary
