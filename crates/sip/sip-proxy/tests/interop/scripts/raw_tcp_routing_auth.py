#!/usr/bin/env python3
"""Drive real-peer TCP routing and authentication-aggregation scenarios."""

from __future__ import annotations

import argparse
import re
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
    request_uri,
    require_external_vias,
    require_safe_value,
    status_and_method,
    write_error,
    write_payload,
)


SCENARIOS = (
    "route-strict",
    "route-loose-record-route",
    "auth-aggregation",
)
AUTH_WWW_CHALLENGES = (
    'Digest realm="origin-a", nonce="origin-a-nonce"',
    'Digest realm="origin-b", nonce="origin-b-nonce"',
    'Digest realm="origin-c", nonce="origin-c-nonce"',
)
AUTH_PROXY_CHALLENGES = (
    'Digest realm="proxy-a", nonce="proxy-a-nonce"',
    'Digest realm="proxy-b", nonce="proxy-b-nonce"',
)


def split_header_entries(message: bytes, header: str) -> list[str]:
    """Flatten controlled SIP list headers without splitting quoted commas."""

    entries: list[str] = []
    for value in header_values(message, header):
        start = 0
        angle_depth = 0
        quoted = False
        escaped = False
        for index, character in enumerate(value):
            if escaped:
                escaped = False
                continue
            if character == "\\" and quoted:
                escaped = True
            elif character == '"':
                quoted = not quoted
            elif not quoted and character == "<":
                angle_depth += 1
            elif not quoted and character == ">":
                angle_depth = max(0, angle_depth - 1)
            elif not quoted and angle_depth == 0 and character == ",":
                entry = value[start:index].strip()
                if entry:
                    entries.append(entry)
                start = index + 1
        entry = value[start:].strip()
        if entry:
            entries.append(entry)
    return entries


def entry_uri(value: str) -> str:
    match = re.search(r"<\s*([^>]+?)\s*>", value)
    if match is not None:
        return match.group(1).strip()
    return value.strip().split(None, 1)[0]


def canonical_uri(value: str) -> str:
    """Canonicalize the controlled interop URI vocabulary for comparison."""

    return re.sub(r"\s+", "", entry_uri(value)).lower()


def uri_entries(message: bytes, header: str) -> list[str]:
    return [
        canonical_uri(value)
        for value in split_header_entries(message, header)
    ]


def remove_lr_parameter(uri: str) -> str:
    """Return the strict-router Request-URI form of a loose-route identity."""

    address, separator, headers = uri.partition("?")
    parts = address.split(";")
    kept = [parts[0], *(part for part in parts[1:] if part.lower() != "lr")]
    stripped = ";".join(kept)
    return stripped + (separator + headers if separator else "")


def require_single_uri(
    observed: list[str],
    expected: str,
    label: str,
) -> None:
    canonical = canonical_uri(expected)
    if observed.count(canonical) != 1:
        raise RuntimeError(
            f"expected exactly one {label} URI {canonical!r}; "
            f"observed={observed!r}"
        )


def require_absent_uri(
    observed: list[str],
    unexpected: str,
    label: str,
) -> None:
    canonical = canonical_uri(unexpected)
    if canonical in observed:
        raise RuntimeError(
            f"{label} unexpectedly retained URI {canonical!r}; "
            f"observed={observed!r}"
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


def routing_observations(
    args: argparse.Namespace,
    downstream: bytes,
    upstream: bytes,
) -> dict[str, Any]:
    downstream_routes = uri_entries(downstream, "Route")
    downstream_record_routes = uri_entries(downstream, "Record-Route")
    upstream_record_routes = uri_entries(upstream, "Record-Route")
    observed_request_uri = canonical_uri(request_uri(downstream))

    if observed_request_uri != canonical_uri(args.original_target_uri):
        raise RuntimeError(
            "route processing did not restore/preserve the original Request-URI: "
            f"expected={canonical_uri(args.original_target_uri)!r} "
            f"observed={observed_request_uri!r}"
        )
    require_single_uri(
        downstream_routes,
        args.next_hop_route_uri,
        "downstream Route",
    )
    require_absent_uri(
        downstream_routes,
        args.rvoip_local_uri,
        "downstream Route",
    )
    require_absent_uri(
        downstream_routes,
        args.original_target_uri,
        "downstream Route",
    )
    require_single_uri(
        downstream_record_routes,
        args.expected_record_route_uri,
        "downstream Record-Route",
    )
    require_single_uri(
        upstream_record_routes,
        args.expected_record_route_uri,
        "upstream Record-Route",
    )
    return {
        "downstream_request_uri": observed_request_uri,
        "downstream_routes": downstream_routes,
        "downstream_record_routes": downstream_record_routes,
        "upstream_record_routes": upstream_record_routes,
        "original_request_uri_preserved": True,
        "local_route_removed": True,
        "next_hop_route_preserved": True,
        "record_route_round_trip": True,
    }


def run_routing(
    args: argparse.Namespace,
    budget: TraceBudget,
) -> dict[str, Any]:
    if len(args.aux_listen) != 1:
        raise ValueError(
            f"{args.scenario} requires exactly one --aux-listen"
        )

    strict = args.scenario == "route-strict"
    if strict:
        initial_uri = remove_lr_parameter(args.expected_record_route_uri)
        phase = "strict-router-recovery"
        route_headers = (
            f"Route: <{args.next_hop_route_uri}>",
            f"Route: <{args.original_target_uri}>",
        )
    else:
        initial_uri = args.original_target_uri
        phase = "loose-route-and-record-route"
        route_headers = (
            (
                f"Route: <{args.rvoip_local_uri}>, "
                f"<{args.next_hop_route_uri}>"
            ),
        )

    with (
        TcpEndpoint(
            "aux-1", args.aux_listen[0], args.timeout, budget
        ) as next_hop,
        TcpUac(args.target, args.uac_bind, args.timeout, budget) as uac,
    ):
        invite = make_request(
            "INVITE",
            args.scenario,
            args.advertised_host,
            uac.local_address[1],
            uri=initial_uri,
            to_value=f"<{args.original_target_uri}>",
            phase=phase,
            extra_headers=route_headers,
        )
        call_id = first_header(invite, "Call-ID")
        uac.send(invite)
        downstream = next_hop.receive()
        assert_request_scenario(downstream.message, args.scenario)
        vias = external_vias(args, downstream.message)
        next_hop.send(
            downstream,
            make_response(downstream.message, 200, "OK"),
        )
        upstream = uac.wait_response(200, "INVITE", call_id=call_id)
        return {
            "external_vias": vias,
            "uac_binding": uac.binding_observation(),
            "upstream_final_status": status_and_method(upstream)[0],
            "routing": routing_observations(
                args,
                downstream.message,
                upstream,
            ),
            "strict_router_recovery": strict,
        }


def challenge_headers(
    name: str, values: tuple[str, ...]
) -> tuple[str, ...]:
    return tuple(f"{name}: {value}" for value in values)


def require_auth_aggregation(response: bytes) -> dict[str, Any]:
    www = header_values(response, "WWW-Authenticate")
    proxy = header_values(response, "Proxy-Authenticate")
    if sorted(www) != sorted(AUTH_WWW_CHALLENGES):
        raise RuntimeError(
            "aggregated WWW-Authenticate values differ from all received "
            f"401 challenges: expected={AUTH_WWW_CHALLENGES!r} "
            f"observed={www!r}"
        )
    if sorted(proxy) != sorted(AUTH_PROXY_CHALLENGES):
        raise RuntimeError(
            "aggregated Proxy-Authenticate values differ from all received "
            f"407 challenges: expected={AUTH_PROXY_CHALLENGES!r} "
            f"observed={proxy!r}"
        )
    return {
        "www_authenticate": www,
        "proxy_authenticate": proxy,
        "www_authenticate_count": len(www),
        "proxy_authenticate_count": len(proxy),
        "mixed_401_407_aggregation": True,
    }


def run_auth_aggregation(
    args: argparse.Namespace,
    budget: TraceBudget,
) -> dict[str, Any]:
    if len(args.aux_listen) != 2:
        raise ValueError(
            "auth-aggregation requires exactly two --aux-listen endpoints"
        )

    with (
        TcpEndpoint(
            "primary", args.primary_listen, args.timeout, budget
        ) as primary,
        TcpEndpoint(
            "aux-1", args.aux_listen[0], args.timeout, budget
        ) as aux_1,
        TcpEndpoint(
            "aux-2", args.aux_listen[1], args.timeout, budget
        ) as aux_2,
        TcpUac(args.target, args.uac_bind, args.timeout, budget) as uac,
    ):
        invite = make_request(
            "INVITE",
            args.scenario,
            args.advertised_host,
            uac.local_address[1],
            phase="parallel-mixed-challenges",
        )
        call_id = first_header(invite, "Call-ID")
        uac.send(invite)
        received = [
            primary.receive(),
            aux_1.receive(),
            aux_2.receive(),
        ]
        endpoints = [primary, aux_1, aux_2]
        via_observations = []
        for item in received:
            assert_request_scenario(item.message, args.scenario)
            via_observations.append(external_vias(args, item.message))

        primary.send(
            received[0],
            make_response(
                received[0].message,
                401,
                "Unauthorized",
                extra_headers=challenge_headers(
                    "WWW-Authenticate",
                    AUTH_WWW_CHALLENGES[:2],
                ),
            ),
        )
        aux_1.send(
            received[1],
            make_response(
                received[1].message,
                401,
                "Unauthorized",
                extra_headers=challenge_headers(
                    "WWW-Authenticate",
                    AUTH_WWW_CHALLENGES[2:],
                ),
            ),
        )
        aux_2.send(
            received[2],
            make_response(
                received[2].message,
                407,
                "Proxy Authentication Required",
                extra_headers=challenge_headers(
                    "Proxy-Authenticate",
                    AUTH_PROXY_CHALLENGES,
                ),
            ),
        )

        upstream = uac.wait_final_response("INVITE", call_id=call_id)
        status, method = status_and_method(upstream)
        if status not in (401, 407) or method != "INVITE":
            raise RuntimeError(
                "authentication aggregation returned an unexpected final: "
                f"status={status} method={method!r}"
            )
        if first_header(upstream, "Call-ID") != call_id:
            raise RuntimeError(
                "authentication aggregation returned the wrong call identity"
            )
        auth = require_auth_aggregation(upstream)
        return {
            "external_vias": via_observations,
            "uac_binding": uac.binding_observation(),
            "upstream_final_status": status,
            "downstream_final_statuses": [401, 401, 407],
            "downstream_branch_count": len(endpoints),
            "authentication": auth,
        }


RUNNERS = {
    "route-strict": run_routing,
    "route-loose-record-route": run_routing,
    "auth-aggregation": run_auth_aggregation,
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
    result.add_argument(
        "--rvoip-local-uri",
        default="sip:rvoip.invalid;transport=tcp;lr",
    )
    result.add_argument(
        "--next-hop-route-uri",
        default="sip:next-hop.invalid;transport=tcp;lr",
    )
    result.add_argument(
        "--original-target-uri",
        default="sip:agent@destination.invalid;transport=tcp",
    )
    result.add_argument(
        "--expected-record-route-uri",
        default="sip:rvoip.invalid;transport=tcp;lr",
    )
    return result


def validate_args(args: argparse.Namespace) -> None:
    if args.timeout <= 0:
        raise ValueError("--timeout must be positive")
    for label in (
        "rvoip_local_uri",
        "next_hop_route_uri",
        "original_target_uri",
        "expected_record_route_uri",
    ):
        value = require_safe_value(
            getattr(args, label),
            f"--{label.replace('_', '-')}",
        )
        if not value.lower().startswith(("sip:", "sips:")):
            raise ValueError(
                f"--{label.replace('_', '-')} must be a SIP URI"
            )
    if ";lr" not in args.rvoip_local_uri.lower():
        raise ValueError("--rvoip-local-uri must contain ;lr")
    if ";lr" not in args.next_hop_route_uri.lower():
        raise ValueError("--next-hop-route-uri must contain ;lr")
    if ";lr" not in args.expected_record_route_uri.lower():
        raise ValueError("--expected-record-route-uri must contain ;lr")


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
