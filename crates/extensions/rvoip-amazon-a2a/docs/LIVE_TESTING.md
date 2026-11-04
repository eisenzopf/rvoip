# Live Connect qualification

The deterministic crate tests do not qualify Amazon Connect interoperability.
Run a real contact through a dedicated Connect instance before claiming a live
A2A pass. A successful `StartWebRTCContact` or media probe alone is insufficient:
it verifies the caller leg, not the external-agent collaboration.

## Text smoke test

1. Follow [AWS's external-agent setup](https://docs.aws.amazon.com/connect/latest/adminguide/a2a-setup-external.html)
   in a dedicated test instance. Configure an `A2A_SERVER` application, a
   customer-managed KMS key and bearer secret, an assistant, an orchestration
   agent with text handoff, and the agent's application allowlist. Associate
   the security profile with both unversioned and published agent ARNs.
2. Associate a published Lex V2 bot containing `AMAZON.QInConnectIntent`.
   Check the **published version's** role, not just the draft bot's role.
   It needs access to the assistant/session and, if applicable, its KMS key.
   See [Lex's permissions](https://docs.aws.amazon.com/lexv2/latest/dg/built-in-intent-qinconnect.html).
   Ensure chat message streaming and `lex:RecognizeMessageAsync` permission
   are enabled.
3. Run `text_echo` behind a public TLS reverse proxy. Set
   `AMAZON_A2A_API_KEY`, `CONNECT_INSTANCE_ARN`, and optionally
   `AMAZON_A2A_PORT` (default `8080`). The listener remains loopback-only.
   Register the resulting `wss://` URL with the application.
4. Publish a test contact flow: enable flow logging; set an English voice;
   associate the assistant (`CreateWisdomSession`); invoke the Lex alias with
   `ConnectParticipantWithLexBot`. Pass `$.Wisdom.SessionArn` as
   `x-amz-lex:q-in-connect:session-arn`. Use the initial prompt
   `Ready for the rvoip integration test.`. After successful completion,
   send `RVOIP_FLOW_COMPLETE` and disconnect. Send a distinct error message
   on the error branch. Configure the orchestrator to call its reserved
   `Complete` tool after the external collaborator finishes.
5. Install `boto3` and `websockets>=14` in a separate Python environment and run:

   ```sh
   python tools/live_chat_smoke.py \
     --profile YOUR_TEST_PROFILE --region YOUR_REGION \
     --instance-id YOUR_INSTANCE_ID --flow-id YOUR_FLOW_ID \
     --output /tmp/a2a-live-result.json
   ```

   Run from this crate's directory. The runner starts an actual Connect chat,
   subscribes to the participant WebSocket, sends a unique synthetic message,
   verifies its exact echo, sends `goodbye`, and requires the goodbye echo and
   the successful flow marker. It exits nonzero on incomplete results. It
   attempts to stop its own contact on every exit and records cleanup status.
   Result files contain status and the contact ID, not tokens or transcripts.
   Use a new output filename for each run.

Enable [assistant event logs](https://docs.aws.amazon.com/connect/latest/adminguide/monitor-ai-agents.html)
and flow logs before testing. Correlate their contact/session IDs with the
adapter endpoint. Preserve sanitized errors and configuration versions.

## Voice qualification

Use the documented Nova Sonic locale and an audio-streaming collaborator with
immediate handoff. The orchestration agent must have no prompt/guardrail and
only the permitted reserved tools. `text_echo` deliberately rejects audio
sessions; a voice test needs an audio-capable host and real turn controller.

Verify both directions of PCM, heard responses, real transcript/tool traces,
interruption, completed artifacts/tasks, and `COMPLETE`/`ESCALATE` hand-back.
For a Vapi bridge, use the existing adapter and qualify the host's event ordering
and upstream audio drain. Do not call a caller-leg WebRTC connection, a local
mock, or a synthetic request to the public endpoint an end-to-end A2A pass.

## Qualification record: 2026-11-04

Region: `us-west-2`. A dedicated instance, assistant, Lex bot, A2A application,
KMS-encrypted secret, security profile, and contact flow were provisioned.

| Check | Result |
| --- | --- |
| Real Connect chat with a baseline orchestration agent and no collaborator | Passed after publishing a dedicated Lex role with assistant/KMS permissions |
| Public TLS adapter endpoint, using a synthetic protocol client | Passed authentication, INIT/READY, text echo, trace, terminal task, and FINISH |
| Native rvoip Connect WebRTC caller leg | Connected; received 528 audio frames during a 20-second probe; contact stopped |
| Live Connect text A2A, immediate and agent-selected handoff | Failed before reaching the external adapter |
| Nova Sonic bot + audio-streaming collaborator, immediate handoff | Voice transport connected; Lex failed its assistant WebSocket upgrade before the external server was invoked |

The Connect flow recorded:

```text
A2A WebSocket upgrade to the Q In Connect Assistant failed (HTTP 404).
```

The corrected baseline received a real assistant response. With the A2A
collaborator configured, the flow failed during Lex processing and the public
adapter endpoint saw no corresponding connection. This narrows the failure to
the AWS integration/runtime path before the external server; it does not establish
whether the cause is configuration, account enablement, or AWS service behavior.
Live A2A interoperability remains **unqualified**. No successful Connect-to-Vapi
end-to-end call is claimed.

A second independently provisioned instance in `us-east-1` reproduced the same
HTTP 404 on a real chat contact. Its published Lex bot used the dedicated IAM
role from its first version. Flow logs confirmed that the assistant session ARN
was resolved and passed to Lex, and message streaming was enabled. The external
adapter received no connection from this contact. An independent authenticated
upgrade to that region's configured public adapter endpoint returned HTTP 101.
The second-region test also remains a failure; endpoint reachability is only a
control check.
The matched `us-east-1` control, with the collaborator removed from the same
agent configuration, received a real assistant response. Its contact was stopped
and the A2A configuration restored afterward.

Further isolation on the same date confirmed `MAX_PACKAGE=true` on both
instances. In `us-east-1`, a temporary region-scoped `wisdom:CreateSession` grant
on `Resource: "*"` did not change the HTTP 404 and was removed. A separate
published agent with `immediateHandoff=false` reproduced the same failure.
A fresh matched built-in-agent control succeeded. Lex conversation logging
captured the successful control but no event for the failed A2A contacts at
collection time; absence of a conversation log is not itself a root cause.
The default agent and temporary IAM/logging changes were restored afterward.
