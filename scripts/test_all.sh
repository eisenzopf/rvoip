#!/usr/bin/env bash
# rvoip Test Suite Runner
# Usage: ./scripts/test_all.sh [level]
#   Levels: unit | adapter | integration | e2e | all (default: all)
#   Examples:
#     ./scripts/test_all.sh          # Run everything
#     ./scripts/test_all.sh unit     # Unit tests only (fastest)
#     ./scripts/test_all.sh adapter  # Adapter roundtrip tests
#     ./scripts/test_all.sh e2e      # End-to-end tests

set -uo pipefail

LEVEL="${1:-all}"
FAILED=0
PASSED=0
SKIPPED=0
ERRORS=()

G='\033[0;32m'; Y='\033[1;33m'; R='\033[0;31m'; B='\033[0;34m'; N='\033[0m'

run() {
    local name="$1"; shift
    printf "${B}[TEST]${N} %-50s " "$name"
    if output=$("$@" 2>&1); then
        result=$(echo "$output" | grep "^test result:" | tail -1)
        printf "${G}PASS${N}  %s\n" "$result"
        ((PASSED++))
    else
        printf "${R}FAIL${N}\n"
        ERRORS+=("$name")
        ((FAILED++))
    fi
}

skip() {
    printf "${B}[SKIP]${N} %-50s ${Y}skipped${N}\n" "$1"
    ((SKIPPED++))
}

CRATES=(sip-core sip-transport dialog-core rtp-core media-core
        session-core client-core call-engine codec-core
        infra-common audio-core sip-client registrar-core)

echo ""
echo -e "${B}rvoip Test Suite${N} (level: $LEVEL)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# ── Level 1: Unit Tests ──
if [[ "$LEVEL" == "unit" || "$LEVEL" == "all" ]]; then
    echo -e "\n${Y}Level 1: Unit Tests${N}"
    for crate in "${CRATES[@]}"; do
        run "unit/$crate" cargo test -p "rvoip-$crate" --lib --no-fail-fast -q
    done
fi

# ── Level 2: Adapter Roundtrip Tests ──
if [[ "$LEVEL" == "adapter" || "$LEVEL" == "all" ]]; then
    echo -e "\n${Y}Level 2: Adapter Roundtrip Tests${N}"
    run "adapter/rtp-packet"  cargo test -p rvoip-rtp-core --lib -q -- packet::adapter
    run "adapter/ice"         cargo test -p rvoip-rtp-core --lib -q -- ice::adapter
    run "adapter/sctp"        cargo test -p rvoip-rtp-core --lib -q -- sctp::adapter
    run "adapter/srtp"        cargo test -p rvoip-rtp-core --lib -q -- srtp::adapter
    run "adapter/dtls"        cargo test -p rvoip-rtp-core --lib -q -- dtls::adapter
    run "adapter/stun"        cargo test -p rvoip-rtp-core --lib -q -- stun::adapter
fi

# ── Level 3: Cross-Module Integration Tests ──
if [[ "$LEVEL" == "integration" || "$LEVEL" == "all" ]]; then
    echo -e "\n${Y}Level 3: Cross-Module Integration Tests${N}"
    run "integration/dtls-srtp"        cargo test -p rvoip-rtp-core --test dtls_srtp_integration -q
    run "integration/dialog-transport" cargo test -p rvoip-integration-tests --test dialog_transport_integration -q
    run "integration/session-media"    cargo test -p rvoip-integration-tests --test session_media_integration -q
    run "integration/ice-sdp"          cargo test -p rvoip-session-core --test ice_sdp_integration -q
fi

# ── Level 4: End-to-End Tests ──
if [[ "$LEVEL" == "e2e" || "$LEVEL" == "all" ]]; then
    echo -e "\n${Y}Level 4: End-to-End Tests${N}"
    run "e2e/call-with-audio"   cargo test -p rvoip-session-core --test e2e_call_with_audio -q
    run "e2e/encrypted-call"    cargo test -p rvoip-session-core --test e2e_encrypted_call -q
    run "e2e/register-and-call" cargo test -p rvoip-session-core --test e2e_register_and_call -q
    run "e2e/transport"         cargo test -p rvoip-sip-transport --test transport_tests -q
    run "e2e/call-center"       cargo test -p rvoip-call-engine --test call_center_tests -q
    run "e2e/b2bua-bridge"      cargo test -p rvoip-call-engine --test b2bua_bridge_test -q
fi

# ── Summary ──
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
printf "Results: ${G}%d passed${N}, ${R}%d failed${N}, ${Y}%d skipped${N}\n" "$PASSED" "$FAILED" "$SKIPPED"

if [[ $FAILED -gt 0 ]]; then
    echo -e "\n${R}Failed tests:${N}"
    for e in "${ERRORS[@]}"; do echo -e "  ${R}x${N} $e"; done
    echo ""
    exit 1
else
    echo -e "${G}All tests passed.${N}"
fi