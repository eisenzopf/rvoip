#!/usr/bin/env python3
"""Exercise an already-provisioned Connect flow and the text_echo example.

Dependencies: boto3 and websockets>=14. Credentials use boto3's normal chain.
Only a synthetic test message is sent. No credentials or transcripts are saved.
"""

import argparse
import json
import time
import uuid
from pathlib import Path

import boto3
from websockets.exceptions import ConnectionClosed
from websockets.sync.client import connect


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile")
    parser.add_argument("--region", required=True)
    parser.add_argument("--instance-id", required=True)
    parser.add_argument("--flow-id", required=True)
    parser.add_argument("--ready-text", default="Ready for the rvoip integration test.")
    parser.add_argument("--complete-text", default="RVOIP_FLOW_COMPLETE")
    parser.add_argument("--timeout", type=float, default=90)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not 1 <= args.timeout <= 300:
        parser.error("--timeout must be between 1 and 300 seconds")
    # Fail before creating a contact if the result cannot be saved.
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.touch(exist_ok=False)
    session = boto3.Session(profile_name=args.profile, region_name=args.region)
    control = session.client("connect")
    participant = session.client("connectparticipant")
    message = "rvoip live test " + uuid.uuid4().hex
    result = {
        "region": args.region,
        "contact_id": None,
        "echo_received": False,
        "goodbye_echo_received": False,
        "flow_complete_received": False,
        "passed": False,
        "failure": None,
        "cleanup": "not_needed",
    }
    connection_token = None
    sent = False
    ended = False
    started = time.monotonic()
    try:
        contact = control.start_chat_contact(
            InstanceId=args.instance_id,
            ContactFlowId=args.flow_id,
            ParticipantDetails={"DisplayName": "rvoip automated smoke test"},
            SupportedMessagingContentTypes=["text/plain"],
        )
        result["contact_id"] = contact["ContactId"]
        connection = participant.create_participant_connection(
            ParticipantToken=contact["ParticipantToken"],
            Type=["WEBSOCKET", "CONNECTION_CREDENTIALS"],
        )
        connection_token = connection["ConnectionCredentials"]["ConnectionToken"]
        with connect(connection["Websocket"]["Url"], open_timeout=10) as socket:
            socket.send(json.dumps({"topic": "aws/subscribe", "content": {"topics": ["aws/chat"]}}))
            while time.monotonic() - started < args.timeout:
                try:
                    event = json.loads(socket.recv(timeout=1))
                except TimeoutError:
                    continue
                except ConnectionClosed:
                    result["failure"] = "participant_socket_closed"
                    break
                if event.get("topic") != "aws/chat":
                    continue
                item = event.get("content", {})
                if isinstance(item, str):
                    item = json.loads(item)
                content_type = item.get("ContentType", "")
                content = item.get("Content", "")
                if content_type == "application/vnd.amazonaws.connect.event.chat.ended":
                    ended = True
                    break
                if content_type == "application/vnd.amazonaws.lex.event.response.failed":
                    result["failure"] = "lex_response_failed"
                    break
                if item.get("ParticipantRole") == "CUSTOMER" or content_type != "text/plain":
                    continue
                if content == args.ready_text and not sent:
                    participant.send_message(ConnectionToken=connection_token, ContentType="text/plain", Content=message)
                    sent = True
                if content == "You said: " + message and not result["echo_received"]:
                    result["echo_received"] = True
                    participant.send_message(ConnectionToken=connection_token, ContentType="text/plain", Content="goodbye")
                if content == "You said: goodbye":
                    result["goodbye_echo_received"] = True
                if content == args.complete_text:
                    result["flow_complete_received"] = True
                    break
            result["passed"] = all(result[key] for key in (
                "echo_received", "goodbye_echo_received", "flow_complete_received"
            ))
            if not result["passed"] and result["failure"] is None:
                result["failure"] = "chat_ended_before_completion" if ended else "timeout_or_incomplete_flow"
    except Exception as error:
        # SDK exception strings may contain request details. Persist only the type/code.
        result["failure"] = getattr(error, "response", {}).get("Error", {}).get("Code", type(error).__name__)
    finally:
        if result["contact_id"]:
            try:
                control.stop_contact(InstanceId=args.instance_id, ContactId=result["contact_id"])
                result["cleanup"] = "stopped"
            except control.exceptions.ContactNotFoundException:
                result["cleanup"] = "already_ended"
            except Exception as error:
                result["cleanup"] = type(error).__name__
                result["passed"] = False
        result["elapsed_seconds"] = round(time.monotonic() - started, 3)
        args.output.write_text(json.dumps(result, indent=2) + "\n")
        print(json.dumps(result))
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
