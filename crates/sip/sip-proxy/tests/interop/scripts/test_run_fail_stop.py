#!/usr/bin/env python3
"""Source-level regression checks for the external interop gate lifecycle."""

from __future__ import annotations

import re
import unittest
from pathlib import Path


RUN_SH = Path(__file__).with_name("run.sh")


def normalized(text: str) -> str:
    return re.sub(r"\s+", " ", text)


class RunFailStopTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.source = RUN_SH.read_text()
        start = cls.source.index("\nrun_row() {")
        end = cls.source.index("\n}\n\nfailures=0", start)
        cls.run_row = cls.source[start:end]
        cls.compact_run_row = normalized(cls.run_row)

    def test_run_row_is_not_called_in_a_conditional_context(self) -> None:
        invocations = [
            line.strip()
            for line in self.source.splitlines()
            if re.search(r"\brun_row\s+\"\$peer\"", line)
        ]
        self.assertEqual(invocations, ['run_row "$peer" "$order" "$transport"'])
        self.assertNotRegex(self.source, r"\bif\s+!?\s*run_row\b")
        self.assertNotRegex(self.source, r"\brun_row\b[^\n]*(?:\|\||&&)")

    def test_required_row_phases_are_explicitly_fail_stop(self) -> None:
        required = (
            "! render_peer_config",
            "! compose config",
            "wait_for_proxy_line RVOIP_PROXY_READY || row_status=FAIL",
            "wait_for_proxy_line 'RVOIP_PROXY_RETENTION phase=pre_zero ' || row_status=FAIL",
            "! docker inspect",
            "! tls_capture_opensips_image_provenance",
            "! compose exec --no-TTY",
            "! assert_peer_version",
            "! start_tls_boundaries",
            "tls_verify_live_endpoints",
            "! run_core_scenarios",
            'run_advanced_scenarios "$target_host" "$target_port" || row_status=FAIL',
            "wait_for_proxy_line 'RVOIP_PROXY_RETENTION phase=activity ' || row_status=FAIL",
            "decode_and_validate_pcaps || row_status=FAIL",
            "wait_for_proxy_line 'RVOIP_PROXY_RETENTION phase=cooldown ' || row_status=FAIL",
            "wait_for_proxy_line 'RVOIP_PROXY_RETENTION phase=post_retention ' || row_status=FAIL",
            'parse_retention "$REQUIRE_RETENTION_CONVERGENCE" || row_status=FAIL',
            'tls_derive_verification_result "$row_dir" "$peer" "$order" || row_status=FAIL',
            'if "$SCRIPT_DIR/down.sh" >"$row_dir/peer-stop.log" 2>&1; then',
        )
        for marker in required:
            with self.subTest(marker=marker):
                self.assertIn(marker, self.compact_run_row)

    def test_successful_row_releases_every_owned_process(self) -> None:
        cleanup_markers = (
            "stop_captures",
            'stop_pid "${uas_pid:-}" TERM',
            'for aux_pid in "${aux_uas_pids[@]:-}"',
            "stop_tls_boundaries",
            '"$SCRIPT_DIR/down.sh"',
            "LAST_ROW_STATUS=$row_status",
        )
        positions = [self.run_row.rindex(marker) for marker in cleanup_markers]
        self.assertEqual(positions, sorted(positions))
        self.assertIn(
            'if "$SCRIPT_DIR/down.sh" >"$row_dir/peer-stop.log" 2>&1; '
            'then active_peer="" else row_status=FAIL fi',
            self.compact_run_row,
        )

    def test_bounded_sipp_wait_retains_live_pid_for_row_cleanup(self) -> None:
        start = self.source.index("\nrun_sipp_scenario() {")
        end = self.source.index("\n}\n\nrun_unmatched_cancel_scenario", start)
        function = normalized(self.source[start:end])
        self.assertIn(
            'if wait_for_background_sipp "$uas_pid"; then uas_pid="" else result=1',
            function,
        )
        self.assertIn(
            'if ! kill -0 "$uas_pid" >/dev/null 2>&1; then uas_pid="" fi',
            function,
        )
        self.assertNotIn(
            'wait_for_background_sipp "$uas_pid" || result=1 uas_pid=""',
            function,
        )

    def test_every_bounded_sipp_wait_preserves_a_still_live_pid(self) -> None:
        self.assertNotRegex(
            self.source,
            r'wait_for_background_sipp "\$uas_pid" \|\| result=1\s+uas_pid=""',
            "a timed-out SIPp process must remain row-owned for cleanup",
        )

    def test_sipp_fallback_outputs_are_contained_in_scenario_evidence(self) -> None:
        self.assertGreaterEqual(
            self.source.count('scenario_dir=$(CDPATH= cd -- "$scenario_dir" && pwd)'),
            2,
        )
        self.assertIn(
            '(cd "$scenario_dir" && exec "$SIPP_BIN" "${sipp_args[@]}")',
            self.source,
        )
        self.assertIn(
            '(cd "$scenario_dir" && "$SIPP_BIN" "${sipp_args[@]}")',
            self.source,
        )

    def test_failed_peer_shutdown_never_erases_peer_ownership(self) -> None:
        cleanup_start = self.source.index("\ncleanup_row() {")
        cleanup_end = self.source.index("\n}\n\ncleanup_all", cleanup_start)
        cleanup = normalized(self.source[cleanup_start:cleanup_end])
        self.assertIn(
            'if "$SCRIPT_DIR/down.sh" >/dev/null 2>&1; then '
            'active_peer="" else echo "failed to stop owned interoperability '
            'peer: $active_peer" >&2 status=1 fi',
            cleanup,
        )
        self.assertNotIn(
            '"$SCRIPT_DIR/down.sh" >/dev/null 2>&1 || true active_peer=""',
            cleanup,
        )
        self.assertIn(
            'if "$SCRIPT_DIR/down.sh" >"$row_dir/peer-stop.log" 2>&1; '
            'then active_peer="" else row_status=FAIL fi',
            self.compact_run_row,
        )

    def test_rfc3263_dns_authority_is_row_owned_and_fail_stop(self) -> None:
        self.assertIn(
            "RFC3263_DNS_PORT=${PROXY_INTEROP_RFC3263_DNS_PORT:-25353}",
            self.source,
        )
        self.assertIn('"$RFC3263_DNS_PORT"', self.source)
        self.assertIn("port $RFC3263_DNS_PORT", self.source)
        self.assertIn(
            '--log "$scenario_dir/dns-queries.jsonl"',
            self.source,
        )
        self.assertIn(
            '--dns-server "$HOST_ADDRESS:$RFC3263_DNS_PORT"',
            self.run_row,
        )
        self.assertIn(
            '--rfc3263-uri "sip:agent@failover.interop.test;transport=tcp"',
            self.run_row,
        )
        start = self.run_row.index("! start_rfc3263_dns")
        proxy = self.run_row.index('"$PROXY_BINARY" "${proxy_args[@]}"')
        self.assertLess(start, proxy)
        self.assertIn("stop_rfc3263_dns || row_status=FAIL", self.run_row)

        cleanup_start = self.source.index("\ncleanup_row() {")
        cleanup_end = self.source.index("\n}\n\ncleanup_all", cleanup_start)
        cleanup = self.source[cleanup_start:cleanup_end]
        self.assertIn("stop_rfc3263_dns || true", cleanup)

    def test_tls_peer_plaintext_egress_socket_is_explicit_and_tracked(self) -> None:
        self.assertIn(
            "PEER_TCP_EGRESS_PORT=${PROXY_INTEROP_PEER_TCP_EGRESS_PORT:-"
            "$((PROXY_INTEROP_PEER_PORT + 1))}",
            self.source,
        )
        self.assertIn('"$PEER_TCP_EGRESS_PORT"', self.source)
        interop = RUN_SH.parent.parent
        for peer, directive, ingress_guard in (
            (
                "opensips",
                "socket=tcp:0.0.0.0:__TCP_EGRESS_PORT__",
                'if ($socket_in(proto) != "tls")',
            ),
            (
                "kamailio",
                "listen=tcp:0.0.0.0:__TCP_EGRESS_PORT__",
                'if ($proto != "tls")',
            ),
        ):
            source = (interop / "config" / f"{peer}-tls.cfg.in").read_text()
            with self.subTest(peer=peer):
                self.assertIn(directive, source)
                self.assertIn(ingress_guard, source)
                if peer == "opensips":
                    self.assertIn(
                        'xlog("L_NOTICE", "INTEROP_TLS_VERIFIED', source
                    )

    def test_dialog_ack_and_bye_routes_are_enabled_end_to_end(self) -> None:
        self.assertIn(
            '--local-uri "sip:$HOST_ADDRESS:$RVOIP_PORT;transport=udp;lr"',
            self.run_row,
        )
        self.assertIn(
            '--record-route-sip "sip:$HOST_ADDRESS:$RVOIP_PORT;transport=udp;lr"',
            normalized(self.run_row),
        )
        self.assertIn(
            '--record-route-sip "sip:$HOST_ADDRESS:$RVOIP_PORT;transport=tcp;lr"',
            normalized(self.run_row),
        )
        self.assertNotIn("sip:rvoip.invalid;transport=tcp;lr", self.source)
        interop = RUN_SH.parent.parent
        for peer in ("kamailio", "opensips"):
            for suffix in ("", "-tls"):
                source = (
                    interop / "config" / f"{peer}{suffix}.cfg.in"
                ).read_text()
                with self.subTest(peer=peer, suffix=suffix):
                    self.assertRegex(
                        normalized(source),
                        r'if \(\(is_method\("ACK"\) \|\| '
                        r'is_method\("BYE"\)\) && loose_route\(\)\)',
                    )

        kamailio_tls = (
            interop / "config" / "kamailio-tls.cfg.in"
        ).read_text()
        transaction_ack = kamailio_tls.index(
            'if (is_method("ACK") && t_check_trans())'
        )
        dialog_route = kamailio_tls.index(
            'if ((is_method("ACK") || is_method("BYE")) && loose_route())'
        )
        self.assertLess(transaction_ack, dialog_route)

        uac = (
            interop / "scenarios" / "invite_success_uac.xml"
        ).read_text()
        uas = (
            interop / "scenarios" / "invite_success_uas.xml"
        ).read_text()
        self.assertIn('response_txn="invite" rtd="true" rrs="true"', uac)
        self.assertIn("ACK [next_url] SIP/2.0", uac)
        self.assertIn("BYE [next_url] SIP/2.0", uac)
        self.assertGreaterEqual(uac.count("[routes]"), 2)
        self.assertGreaterEqual(uas.count("[last_Record-Route:]"), 2)

    def test_core_scenarios_and_tls_result_are_fail_stop(self) -> None:
        core_start = self.source.index("\nrun_core_scenarios() {")
        core_end = self.source.index("\n}\n\nadvanced_scenarios_for_row", core_start)
        core = normalized(self.source[core_start:core_end])
        for scenario in (
            "run_matched_cancel_scenario before",
            "run_matched_cancel_scenario after",
            "run_cancel_retransmission_scenario",
            "run_unmatched_cancel_scenario",
            "message-body-content-length",
        ):
            self.assertIn(scenario, core)
        self.assertRegex(
            core,
            r"message-body-content-length .* \"\$target_host\" \"\$target_port\"",
        )
        self.assertIn('SCENARIO_EVIDENCE" tls', self.run_row)
        self.assertIn("|| row_status=FAIL", self.run_row)

    def test_each_core_scenario_owns_its_packet_capture(self) -> None:
        wrapper_start = self.source.index("\nrun_captured_external_scenario() {")
        wrapper_end = self.source.index("\n}\n\nsipp_mode", wrapper_start)
        wrapper = normalized(self.source[wrapper_start:wrapper_end])
        self.assertIn('start_captures "$scenario" || return', wrapper)
        self.assertIn("stop_captures", wrapper)
        self.assertIn('validate_scenario_packet_evidence "$scenario"', wrapper)

        core_start = self.source.index("\nrun_core_scenarios() {")
        core_end = self.source.index("\n}\n\nadvanced_scenarios_for_row", core_start)
        core = self.source[core_start:core_end]
        for scenario in (
            "options-readiness",
            "invite-success",
            "matched-cancel-before-provisional",
            "matched-cancel-after-provisional",
            "cancel-retransmission",
            "unmatched-cancel",
            "ack-non2xx",
            "via-response-destination",
            "message-body-content-length",
        ):
            with self.subTest(scenario=scenario):
                self.assertIn(f"run_captured_external_scenario {scenario}", core)

        self.assertIn('"$row_dir/${scenario}--$safe_name.pcap"', self.source)
        self.assertNotIn('"$row_dir/$safe_name.pcap"', self.source)

    def test_packet_capture_uses_explicit_loss_resistant_buffer(self) -> None:
        self.assertIn(
            "TCPDUMP_BUFFER_KIB=${PROXY_INTEROP_TCPDUMP_BUFFER_KIB:-32768}",
            self.source,
        )
        self.assertIn(
            '-B "$TCPDUMP_BUFFER_KIB" -i "$interface"',
            self.source,
        )

    def test_tcp_uac_uses_per_call_socket_while_uas_remains_single_socket(
        self,
    ) -> None:
        self.assertIn("sipp_uac_mode()", self.source)
        self.assertIn("tcp|tls) printf '%s\\n' tn", self.source)
        self.assertIn('mode=$(sipp_uac_mode "$current_transport")', self.source)
        self.assertIn('mode=$(sipp_mode "$current_transport")', self.source)

    def test_raw_tcp_core_scenarios_do_not_reuse_uac_source_ports(
        self,
    ) -> None:
        for variable in (
            "RAW_UAC_PORT_MATCHED_BEFORE",
            "RAW_UAC_PORT_MATCHED_AFTER",
            "RAW_UAC_PORT_CANCEL_RETRANSMISSION",
            "RAW_UAC_PORT_UNMATCHED_CANCEL",
        ):
            with self.subTest(variable=variable):
                self.assertIn(variable, self.source)
                self.assertIn(f'"${variable}"', self.source)
        self.assertGreaterEqual(
            self.source.count('if [[ "$current_transport" != udp ]]'),
            3,
        )

    def test_capacity_probe_window_leaves_short_timer_c_cleanup_margin(
        self,
    ) -> None:
        advanced_start = self.source.index("\nrun_advanced_scenarios() {")
        advanced_end = self.source.index("\ncurrent_order=", advanced_start)
        advanced = normalized(self.source[advanced_start:advanced_end])
        capacity_start = advanced.index("capacity-overload)")
        capacity_end = advanced.index("route-strict|", capacity_start)
        capacity = advanced[capacity_start:capacity_end]
        self.assertIn("--capacity-fill-limit 72", capacity)
        self.assertIn("--quiet-window-ms 50", capacity)

    def test_signal_cleanup_exits_and_pki_is_trap_owned(self) -> None:
        exit_trap_marker = (
            'trap \'status=$?; trap - EXIT; cleanup_all "$status"; '
            'exit "$status"\' EXIT'
        )
        exit_trap = self.source.index(exit_trap_marker)
        int_trap = self.source.index("trap 'cleanup_signal INT' INT")
        term_trap = self.source.index("trap 'cleanup_signal TERM' TERM")
        prepare = self.source.index('tls_prepare_pki "$WORKSPACE_ROOT"')
        self.assertLess(exit_trap, prepare)
        self.assertLess(int_trap, prepare)
        self.assertLess(term_trap, prepare)
        self.assertIn("INT) status=130", self.source)
        self.assertIn("TERM) status=143", self.source)
        self.assertIn('exit "$status"', self.source)

    def test_artifacts_and_proxy_binary_are_bound_fail_closed(self) -> None:
        self.assertIn(
            "artifact directory must be a fresh, empty directory", self.source
        )
        self.assertIn("proxy-binary.sha256", self.source)
        self.assertIn("proxy-binary.path", self.source)
        self.assertIn("proxy-binary-check.txt", self.source)
        self.assertIn("cargo-build-command.txt", self.source)
        self.assertIn('if [[ "$proxy_binary_unchanged" != true ]]', self.source)
        self.assertIn("runtime-state-settle.log", self.source)
        self.assertIn("sipp_path:", self.source)
        self.assertIn("first_nonempty_line", self.source)


if __name__ == "__main__":
    unittest.main()
