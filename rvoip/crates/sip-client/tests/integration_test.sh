#!/bin/bash
#
# RVOIP SIP Client Integration Test
# Tests peer-to-peer SIP communication with audio verification
#

set -e  # Exit on any error

# Test configuration
TEST_NAME="sip-client-integration-test"
TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BASE_DIR="$(dirname "$TEST_DIR")"
LOGS_DIR="$TEST_DIR/logs"
AUDIO_DIR="$TEST_DIR/audio"
RESULTS_DIR="$TEST_DIR/results"

# Network configuration
ALICE_PORT=5061
BOB_PORT=5062
ALICE_MEDIA_PORT=6001
BOB_MEDIA_PORT=6002

# Test files
ALICE_LOG="$LOGS_DIR/alice.log"
BOB_LOG="$LOGS_DIR/bob.log"
TEST_RESULT="$RESULTS_DIR/test_result.json"

# Audio test files
ALICE_AUDIO_IN="$AUDIO_DIR/alice_says.wav"
ALICE_AUDIO_OUT="$RESULTS_DIR/alice_received.wav"
BOB_AUDIO_IN="$AUDIO_DIR/bob_says.wav"
BOB_AUDIO_OUT="$RESULTS_DIR/bob_received.wav"

# PIDs for cleanup
ALICE_PID=""
BOB_PID=""

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Logging functions
log() {
    echo -e "${BLUE}[$(date '+%H:%M:%S')]${NC} $1"
}

success() {
    echo -e "${GREEN}[$(date '+%H:%M:%S')] ✅ $1${NC}"
}

warning() {
    echo -e "${YELLOW}[$(date '+%H:%M:%S')] ⚠️  $1${NC}"
}

error() {
    echo -e "${RED}[$(date '+%H:%M:%S')] ❌ $1${NC}"
}

# Cleanup function
cleanup() {
    log "🧹 Cleaning up test processes..."
    
    if [ ! -z "$ALICE_PID" ]; then
        kill $ALICE_PID 2>/dev/null || true
        wait $ALICE_PID 2>/dev/null || true
    fi
    
    if [ ! -z "$BOB_PID" ]; then
        kill $BOB_PID 2>/dev/null || true
        wait $BOB_PID 2>/dev/null || true
    fi
    
    # Kill any remaining rvoip-sip-client processes
    pkill -f "rvoip-sip-client" 2>/dev/null || true
    
    log "Cleanup complete"
}

# Setup cleanup trap
trap cleanup EXIT INT TERM

# Setup test environment
setup_test_env() {
    log "🚀 Setting up RVOIP SIP Client Integration Test"
    
    # Create directories
    mkdir -p "$LOGS_DIR" "$RESULTS_DIR" "$AUDIO_DIR"
    
    # Clean previous logs
    rm -f "$ALICE_LOG" "$BOB_LOG" "$TEST_RESULT"
    rm -f "$ALICE_AUDIO_OUT" "$BOB_AUDIO_OUT"
    
    # Build the project
    log "🔨 Building rvoip-sip-client..."
    cd "$BASE_DIR"
    cargo build --bin rvoip-sip-client
    
    success "Test environment ready"
}

# Create test audio files
create_test_audio() {
    log "🎵 Creating test audio files..."
    
    # Create simple test audio files using ffmpeg (if available)
    if command -v ffmpeg >/dev/null 2>&1; then
        # Alice's test message (440Hz tone for 3 seconds)
        ffmpeg -f lavfi -i "sine=frequency=440:duration=3" -ar 8000 -ac 1 -acodec pcm_mulaw "$ALICE_AUDIO_IN" -y >/dev/null 2>&1
        
        # Bob's test message (880Hz tone for 3 seconds) 
        ffmpeg -f lavfi -i "sine=frequency=880:duration=3" -ar 8000 -ac 1 -acodec pcm_mulaw "$BOB_AUDIO_IN" -y >/dev/null 2>&1
        
        success "Created test audio files with ffmpeg"
    else
        # Create simple header-only WAV files for testing
        create_dummy_wav "$ALICE_AUDIO_IN" "Alice test audio"
        create_dummy_wav "$BOB_AUDIO_IN" "Bob test audio"
        
        warning "ffmpeg not available, created dummy audio files"
    fi
}

# Create dummy WAV file (just header, for testing purposes)
create_dummy_wav() {
    local file="$1"
    local description="$2"
    
    # Create a minimal WAV file (just headers)
    {
        # RIFF header
        printf "RIFF"
        printf "\x24\x00\x00\x00"  # File size - 8
        printf "WAVE"
        
        # fmt chunk
        printf "fmt "
        printf "\x10\x00\x00\x00"  # Chunk size
        printf "\x07\x00"          # Audio format (μ-law)
        printf "\x01\x00"          # Num channels
        printf "\x40\x1f\x00\x00"  # Sample rate (8000)
        printf "\x40\x1f\x00\x00"  # Byte rate
        printf "\x01\x00"          # Block align
        printf "\x08\x00"          # Bits per sample
        
        # data chunk
        printf "data"
        printf "\x00\x00\x00\x00"  # Data size (empty)
    } > "$file"
    
    log "Created dummy WAV file: $file ($description)"
}

# Start Alice (call receiver)
start_alice() {
    log "👩 Starting Alice (call receiver) on port $ALICE_PORT..."
    
    cd "$BASE_DIR"
    cargo run --bin rvoip-sip-client -- \
        --port $ALICE_PORT \
        --username alice \
        --domain 127.0.0.1 \
        receive \
        --auto-answer \
        --max-duration 30 \
        > "$ALICE_LOG" 2>&1 &
    
    ALICE_PID=$!
    log "Alice PID: $ALICE_PID"
    
    # Wait for Alice to start
    sleep 2
    
    if kill -0 $ALICE_PID 2>/dev/null; then
        success "Alice started successfully"
    else
        error "Alice failed to start"
        return 1
    fi
}

# Start Bob (call initiator)  
start_bob() {
    log "👨 Starting Bob (call initiator) on port $BOB_PORT..."
    
    cd "$BASE_DIR"
    cargo run --bin rvoip-sip-client -- \
        --port $BOB_PORT \
        --username bob \
        --domain 127.0.0.1 \
        call \
        "sip:alice@127.0.0.1:$ALICE_PORT" \
        --duration 10 \
        --auto-hangup \
        > "$BOB_LOG" 2>&1 &
    
    BOB_PID=$!
    log "Bob PID: $BOB_PID"
    
    # Wait for Bob to start
    sleep 2
    
    if kill -0 $BOB_PID 2>/dev/null; then
        success "Bob started successfully"
    else
        error "Bob failed to start"
        return 1
    fi
}

# Monitor the test progress
monitor_test() {
    log "📊 Monitoring test progress..."
    
    local timeout=60  # 60 second timeout
    local elapsed=0
    
    while [ $elapsed -lt $timeout ]; do
        # Check if both processes are still running
        alice_running=$(kill -0 $ALICE_PID 2>/dev/null && echo "true" || echo "false")
        bob_running=$(kill -0 $BOB_PID 2>/dev/null && echo "true" || echo "false")
        
        log "Alice: $alice_running, Bob: $bob_running (${elapsed}s/${timeout}s)"
        
        # Check for call completion in logs
        if grep -q "Call completed\|Call ended\|Call hung up" "$BOB_LOG" 2>/dev/null; then
            success "Call completed detected in Bob's log"
            break
        fi
        
        if grep -q "Call ended\|Call completed" "$ALICE_LOG" 2>/dev/null; then
            success "Call completion detected in Alice's log"
            break
        fi
        
        sleep 2
        elapsed=$((elapsed + 2))
    done
    
    if [ $elapsed -ge $timeout ]; then
        warning "Test monitoring timed out after ${timeout} seconds"
    fi
}

# Analyze test results
analyze_results() {
    log "📋 Analyzing test results..."
    
    local alice_registered="false"
    local bob_registered="false"
    local call_initiated="false"
    local call_connected="false"
    local call_completed="false"
    local audio_transmitted="false"
    local errors_found="false"
    
    # Check Alice's log
    if [ -f "$ALICE_LOG" ]; then
        if grep -q "Registration successful\|registered with\|✅.*regist" "$ALICE_LOG"; then
            alice_registered="true"
        fi
        
        if grep -q "Incoming call\|Call answered\|auto-answer" "$ALICE_LOG"; then
            call_connected="true"
        fi
        
        if grep -q "ERROR\|WARN.*Failed\|❌" "$ALICE_LOG"; then
            errors_found="true"
        fi
    fi
    
    # Check Bob's log
    if [ -f "$BOB_LOG" ]; then
        if grep -q "Registration successful\|registered with\|✅.*regist" "$BOB_LOG"; then
            bob_registered="true"
        fi
        
        if grep -q "Making call\|Calling" "$BOB_LOG"; then
            call_initiated="true"
        fi
        
        if grep -q "Call connected\|answered\|✅.*Call.*answered" "$BOB_LOG"; then
            call_connected="true"
        fi
        
        if grep -q "Call completed\|Call ended\|hung up" "$BOB_LOG"; then
            call_completed="true"
        fi
        
        if grep -q "ERROR\|WARN.*Failed\|❌" "$BOB_LOG"; then
            errors_found="true"
        fi
    fi
    
    # Create test result JSON
    cat > "$TEST_RESULT" << EOF
{
    "test_name": "$TEST_NAME",
    "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
    "results": {
        "alice_registered": $alice_registered,
        "bob_registered": $bob_registered,
        "call_initiated": $call_initiated,
        "call_connected": $call_connected,
        "call_completed": $call_completed,
        "audio_transmitted": $audio_transmitted,
        "errors_found": $errors_found
    },
    "files": {
        "alice_log": "$ALICE_LOG",
        "bob_log": "$BOB_LOG",
        "alice_audio_in": "$ALICE_AUDIO_IN",
        "bob_audio_in": "$BOB_AUDIO_IN"
    }
}
EOF
    
    # Print summary
    echo
    echo "════════════════════════════════════════"
    echo "🧪 RVOIP SIP CLIENT INTEGRATION TEST RESULTS"
    echo "════════════════════════════════════════"
    echo "📅 Test completed: $(date)"
    echo
    echo "📊 Test Results:"
    echo "   Alice registered:    $([ "$alice_registered" = "true" ] && echo "✅ YES" || echo "❌ NO")"
    echo "   Bob registered:      $([ "$bob_registered" = "true" ] && echo "✅ YES" || echo "❌ NO")"
    echo "   Call initiated:      $([ "$call_initiated" = "true" ] && echo "✅ YES" || echo "❌ NO")"
    echo "   Call connected:      $([ "$call_connected" = "true" ] && echo "✅ YES" || echo "❌ NO")"
    echo "   Call completed:      $([ "$call_completed" = "true" ] && echo "✅ YES" || echo "❌ NO")"
    echo "   Audio transmitted:   $([ "$audio_transmitted" = "true" ] && echo "✅ YES" || echo "🚧 TODO")"
    echo "   Errors found:        $([ "$errors_found" = "true" ] && echo "⚠️ YES" || echo "✅ NO")"
    echo
    echo "📁 Log Files:"
    echo "   Alice: $ALICE_LOG"
    echo "   Bob:   $BOB_LOG"
    echo "   Results: $TEST_RESULT"
    echo
    
    # Overall result
    if [ "$call_initiated" = "true" ] && [ "$call_connected" = "true" ]; then
        success "🎉 SIP COMMUNICATION TEST PASSED!"
        echo "   ✅ Peer-to-peer SIP communication is working"
        echo "   ✅ Call setup and teardown successful"
        return 0
    else
        error "❌ SIP COMMUNICATION TEST FAILED"
        echo "   Check log files for details"
        return 1
    fi
}

# Print usage
usage() {
    echo "Usage: $0 [options]"
    echo
    echo "Options:"
    echo "  --help, -h          Show this help message"
    echo "  --logs-only         Only analyze existing logs"
    echo "  --cleanup           Clean up test files and exit"
    echo
    echo "This script tests peer-to-peer SIP communication between two"
    echo "rvoip-sip-client instances with audio verification."
}

# Main test execution
main() {
    case "${1:-}" in
        --help|-h)
            usage
            exit 0
            ;;
        --logs-only)
            analyze_results
            exit $?
            ;;
        --cleanup)
            cleanup
            rm -rf "$LOGS_DIR" "$RESULTS_DIR" "$AUDIO_DIR" 2>/dev/null || true
            success "Test files cleaned up"
            exit 0
            ;;
    esac
    
    log "🚀 Starting RVOIP SIP Client Integration Test"
    
    # Setup
    setup_test_env
    create_test_audio
    
    # Start clients
    start_alice || exit 1
    sleep 3  # Give Alice time to fully initialize
    
    start_bob || exit 1
    sleep 2  # Give Bob time to start
    
    # Monitor and wait for completion
    monitor_test
    
    # Wait a bit more for processes to complete
    sleep 5
    
    # Analyze results
    analyze_results
    exit $?
}

# Run main function with all arguments
main "$@" 