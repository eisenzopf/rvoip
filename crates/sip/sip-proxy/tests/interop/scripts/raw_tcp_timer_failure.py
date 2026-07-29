#!/usr/bin/env python3
"""Drive real-peer TCP Timer C, failover, failure, and overload scenarios."""

from __future__ import annotations

import argparse
import time
from pathlib import Path
from typing import Any

from raw_tcp_advanced_support import (
    TcpEndpoint,
    TcpUac,
    TraceBudget,
    assert_request_scenario,
    finalize_payload,
    first_header,
    header_values,
    make_request,
    make_response,
    parse_address,
    parse_bind_address,
    require_external_vias,
    status_and_method,
    via_branch,
    write_error,
    write_payload,
)


SCENARIOS = (
    "timer-c-calling",
    "timer-c-proceeding",
    "transport-failure",
    "rfc3263-failover",
    "capacity-overload",
)


def external_vias(
    args: argparse.Namespace, message: bytes
) -> dict[str, Any]:
    return require_external_vias(
        message,
        args.expected_rvoip_sent_by,
        args.expected_peer_sent_by,
        args.order,
    )


def new_invite(
    args: argparse.Namespace,
    phase: str,
    advertised_port: int,
    *,
    call_id: str | None = None,
) -> bytes:
    return make_request(
        "INVITE",
        args.scenario,
        args.advertised_host,
        advertised_port,
        call_id=call_id,
        phase=phase,
    )


def run_probe(
    args: argparse.Namespace,
    uac: TcpUac,
    primary: TcpEndpoint,
) -> dict[str, Any]:
    request = make_request(
        "OPTIONS",
        args.scenario,
        args.advertised_host,
        uac.local_address[1],
        phase="external-traversal-probe",
    )
    call_id = first_header(request, "Call-ID")
    uac.send(request)
    downstream = primary.receive()
    assert_request_scenario(downstream.message, args.scenario)
    vias = external_vias(args, downstream.message)
    primary.send(
        downstream,
        make_response(downstream.message, 200, "OK"),
    )
    uac.wait_response(200, "OPTIONS", call_id=call_id)
    return vias


def timer_bounds(
    args: argparse.Namespace, elapsed_seconds: float
) -> dict[str, Any]:
    configured = args.timer_c_ms / 1000.0
    minimum = configured * 0.5
    maximum = min(args.timeout, max(configured * 5.0, configured + 1.0))
    if not minimum <= elapsed_seconds <= maximum:
        raise RuntimeError(
            "Timer C completion fell outside the bounded test window: "
            f"configured={configured:.3f}s elapsed={elapsed_seconds:.3f}s "
            f"bounds=[{minimum:.3f},{maximum:.3f}]"
        )
    return {
        "configured_ms": args.timer_c_ms,
        "elapsed_ms": round(elapsed_seconds * 1000, 3),
        "minimum_ms": round(minimum * 1000, 3),
        "maximum_ms": round(maximum * 1000, 3),
    }


def run_timer_c_calling(
    args: argparse.Namespace,
    budget: TraceBudget,
) -> dict[str, Any]:
    with (
        TcpEndpoint(
            "primary", args.primary_listen, args.timeout, budget
        ) as primary,
        TcpUac(args.target, args.uac_bind, args.timeout, budget) as uac,
    ):
        invite = new_invite(
            args,
            "calling-no-provisional",
            uac.local_address[1],
        )
        call_id = first_header(invite, "Call-ID")
        started = time.monotonic()
        uac.send(invite)
        downstream = primary.receive()
        assert_request_scenario(downstream.message, args.scenario)
        vias = external_vias(args, downstream.message)
        final = uac.wait_response(408, "INVITE", call_id=call_id)
        elapsed = time.monotonic() - started
        if primary.receive_optional(args.quiet_window_ms / 1000.0) is not None:
            raise RuntimeError(
                "Timer C Calling scenario emitted an unexpected downstream request"
            )
        return {
            "external_vias": vias,
            "timer_c": timer_bounds(args, elapsed),
            "uac_binding": uac.binding_observation(),
            "upstream_final_status": status_and_method(final)[0],
            "downstream_provisional_responses": 0,
        }


def run_timer_c_proceeding(
    args: argparse.Namespace,
    budget: TraceBudget,
) -> dict[str, Any]:
    with (
        TcpEndpoint(
            "primary", args.primary_listen, args.timeout, budget
        ) as primary,
        TcpUac(args.target, args.uac_bind, args.timeout, budget) as uac,
    ):
        invite = new_invite(
            args,
            "proceeding-then-expire",
            uac.local_address[1],
        )
        call_id = first_header(invite, "Call-ID")
        uac.send(invite)
        downstream_invite = primary.receive()
        assert_request_scenario(
            downstream_invite.message, args.scenario
        )
        vias = external_vias(args, downstream_invite.message)
        primary.send(
            downstream_invite,
            make_response(downstream_invite.message, 180, "Ringing"),
        )
        uac.wait_response(180, "INVITE", call_id=call_id)
        started = time.monotonic()
        downstream_cancel = primary.receive()
        elapsed = time.monotonic() - started
        if not downstream_cancel.message.startswith(b"CANCEL "):
            raise RuntimeError("Timer C Proceeding did not generate CANCEL")
        if (
            first_header(downstream_cancel.message, "Call-ID") != call_id
            or via_branch(downstream_cancel.message)
            != via_branch(downstream_invite.message)
        ):
            raise RuntimeError(
                "Timer C generated CANCEL does not match the INVITE transaction"
            )
        primary.send(
            downstream_cancel,
            make_response(downstream_cancel.message, 200, "OK"),
        )
        primary.send(
            downstream_invite,
            make_response(
                downstream_invite.message,
                487,
                "Request Terminated",
            ),
        )
        final = uac.wait_response(487, "INVITE", call_id=call_id)
        duplicate = primary.receive_optional(
            args.quiet_window_ms / 1000.0
        )
        if duplicate is not None and duplicate.message.startswith(b"CANCEL "):
            raise RuntimeError("Timer C generated duplicate downstream CANCEL")
        return {
            "external_vias": vias,
            "timer_c": timer_bounds(args, elapsed),
            "uac_binding": uac.binding_observation(),
            "upstream_final_status": status_and_method(final)[0],
            "downstream_cancel_requests": 1,
            "cancel_branch": via_branch(downstream_cancel.message),
        }


def run_transport_failure(
    args: argparse.Namespace,
    budget: TraceBudget,
) -> dict[str, Any]:
    with (
        TcpEndpoint(
            "primary", args.primary_listen, args.timeout, budget
        ) as primary,
        TcpUac(args.target, args.uac_bind, args.timeout, budget) as uac,
    ):
        vias = run_probe(args, uac, primary)
        invite = new_invite(
            args,
            "route-to-unreachable-target",
            uac.local_address[1],
        )
        call_id = first_header(invite, "Call-ID")
        uac.send(invite)
        final = uac.wait_response(
            args.expected_failure_status,
            "INVITE",
            call_id=call_id,
        )
        if primary.receive_optional(args.quiet_window_ms / 1000.0) is not None:
            raise RuntimeError(
                "transport-failure INVITE reached the normal live target"
            )
        return {
            "external_vias": vias,
            "uac_binding": uac.binding_observation(),
            "upstream_final_status": status_and_method(final)[0],
            "normal_target_received_failed_invite": False,
        }


def run_rfc3263_failover(
    args: argparse.Namespace,
    budget: TraceBudget,
) -> dict[str, Any]:
    if len(args.aux_listen) != 1:
        raise ValueError(
            "rfc3263-failover requires exactly one --aux-listen live target"
        )
    with (
        TcpEndpoint(
            "aux-1", args.aux_listen[0], args.timeout, budget
        ) as live_target,
        TcpUac(args.target, args.uac_bind, args.timeout, budget) as uac,
    ):
        invite = new_invite(
            args,
            "ordered-dead-then-live",
            uac.local_address[1],
        )
        call_id = first_header(invite, "Call-ID")
        uac.send(invite)
        downstream = live_target.receive()
        assert_request_scenario(downstream.message, args.scenario)
        vias = external_vias(args, downstream.message)
        live_target.send(
            downstream,
            make_response(downstream.message, 200, "OK"),
        )
        final = uac.wait_response(200, "INVITE", call_id=call_id)
        return {
            "external_vias": vias,
            "uac_binding": uac.binding_observation(),
            "upstream_final_status": status_and_method(final)[0],
            "live_candidate_index": 2,
            "live_target_received_invites": 1,
        }


def expected_overload_status(args: argparse.Namespace) -> int:
    if args.expected_overload_status is not None:
        return args.expected_overload_status
    return 500 if args.order == "peer-first" else 503


def release_held_calls(
    primary: TcpEndpoint,
    uac: TcpUac,
    held: list[tuple[str, Any]],
) -> int:
    for _call_id, downstream in held:
        primary.send(
            downstream,
            make_response(downstream.message, 486, "Busy Here"),
        )

    pending = {call_id for call_id, _downstream in held}
    while pending:
        final = uac.wait_final_response("INVITE")
        status, _method = status_and_method(final)
        call_id = first_header(final, "Call-ID")
        if call_id not in pending:
            raise RuntimeError(
                "capacity-overload received an unexpected or duplicate "
                f"final response for {call_id!r}"
            )
        if status != 486:
            raise RuntimeError(
                "capacity-overload held call did not finish with 486: "
                f"call_id={call_id!r} status={status}"
            )
        pending.remove(call_id)
    return len(held)


def run_capacity_overload(
    args: argparse.Namespace,
    budget: TraceBudget,
) -> dict[str, Any]:
    overload_status = expected_overload_status(args)
    with (
        TcpEndpoint(
            "primary", args.primary_listen, args.timeout, budget
        ) as primary,
        TcpUac(args.target, args.uac_bind, args.timeout, budget) as uac,
    ):
        held: list[tuple[str, Any]] = []
        vias: dict[str, Any] | None = None
        overloaded: bytes | None = None
        for index in range(args.capacity_fill_limit):
            request = new_invite(
                args,
                f"hold-capacity-{index + 1}",
                uac.local_address[1],
            )
            call_id = first_header(request, "Call-ID")
            uac.send(request)
            downstream = primary.receive_optional(
                args.quiet_window_ms / 1000.0
            )
            if downstream is not None:
                assert_request_scenario(downstream.message, args.scenario)
                observed_vias = external_vias(args, downstream.message)
                vias = vias or observed_vias
                held.append((call_id, downstream))
                continue
            overloaded = uac.wait_response(
                overload_status,
                "INVITE",
                call_id=call_id,
            )
            break
        if overloaded is None:
            raise RuntimeError(
                "capacity-overload did not reach the response-context limit "
                f"within {args.capacity_fill_limit} attempts"
            )
        if not held or vias is None:
            raise RuntimeError(
                "capacity-overload did not create a live held response context "
                "before rejecting work"
            )
        retry_after = header_values(overloaded, "Retry-After")
        released = release_held_calls(primary, uac, held)
        return {
            "external_vias": vias,
            "uac_binding": uac.binding_observation(),
            "overload_status": status_and_method(overloaded)[0],
            "retry_after": retry_after,
            "retry_after_present": bool(retry_after),
            "held_contexts_created": len(held),
            "held_calls_released": released,
            "capacity_fill_limit": args.capacity_fill_limit,
            "overloaded_call_reached_target": False,
        }


RUNNERS = {
    "timer-c-calling": run_timer_c_calling,
    "timer-c-proceeding": run_timer_c_proceeding,
    "transport-failure": run_transport_failure,
    "rfc3263-failover": run_rfc3263_failover,
    "capacity-overload": run_capacity_overload,
}


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--scenario", choices=SCENARIOS, required=True)
    result.add_argument(
        "--primary-listen", type=parse_address, required=True
    )
    result.add_argument(
        "--aux-listen", type=parse_address, action="append", default=[]
    )
    result.add_argument("--uac-bind", type=parse_bind_address, required=True)
    result.add_argument("--target", type=parse_address, required=True)
    result.add_argument("--advertised-host", required=True)
    result.add_argument("--expected-rvoip-sent-by", required=True)
    result.add_argument("--expected-peer-sent-by", required=True)
    result.add_argument(
        "--order",
        choices=("rvoip-first", "peer-first"),
        default="peer-first",
    )
    result.add_argument("--output-dir", type=Path, required=True)
    result.add_argument("--timeout", type=float, default=10.0)
    result.add_argument("--timer-c-ms", type=int, default=500)
    result.add_argument("--quiet-window-ms", type=int, default=250)
    result.add_argument("--expected-failure-status", type=int, default=500)
    result.add_argument("--expected-overload-status", type=int)
    result.add_argument("--capacity-fill-limit", type=int, default=72)
    return result


def validate_args(args: argparse.Namespace) -> None:
    if args.timeout <= 0:
        raise ValueError("--timeout must be positive")
    if args.timer_c_ms <= 0:
        raise ValueError("--timer-c-ms must be positive")
    if args.quiet_window_ms <= 0:
        raise ValueError("--quiet-window-ms must be positive")
    if not 2 <= args.capacity_fill_limit <= 128:
        raise ValueError("--capacity-fill-limit must be in 2..128")
    for label in (
        "expected_failure_status",
        "expected_overload_status",
    ):
        value = getattr(args, label)
        if value is None and label == "expected_overload_status":
            continue
        if not 400 <= value <= 699:
            raise ValueError(f"--{label.replace('_', '-')} must be 400..699")


def main() -> int:
    args = parser().parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    budget = TraceBudget(args.output_dir)
    try:
        validate_args(args)
        observations = RUNNERS[args.scenario](args, budget)
        write_payload(
            args.output_dir,
            finalize_payload(
                args.output_dir,
                args.scenario,
                observations,
                budget,
            ),
        )
        return 0
    except (OSError, RuntimeError, TimeoutError, ValueError) as error:
        write_error(args.output_dir, error)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
