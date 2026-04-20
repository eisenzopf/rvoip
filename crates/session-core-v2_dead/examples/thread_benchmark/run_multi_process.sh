#!/bin/bash
# Multi-process benchmark: 1 UAS + 5 UAC in separate processes

set -e

# Color codes
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/../../../.."

echo -e "${BLUE}=== Building benchmark binaries ===${NC}"
cargo build --release --example uas_answerer -p rvoip-session-core-v2
cargo build --release --example uac_caller -p rvoip-session-core-v2

echo ""
echo -e "${BLUE}=== Starting UAS Answerer (port 6000) ===${NC}"
./target/release/examples/uas_answerer > /tmp/uas_answerer.log 2>&1 &
UAS_PID=$!
echo -e "UAS PID: ${GREEN}$UAS_PID${NC}"

# Wait for UAS to start
sleep 2

echo ""
echo -e "${BLUE}=== Starting 5 UAC Callers (ports 6001-6005) ===${NC}"
CALLER_PIDS=()
for i in {0..4}; do
    ./target/release/examples/uac_caller $i > /tmp/uac_caller_$i.log 2>&1 &
    CALLER_PID=$!
    CALLER_PIDS+=($CALLER_PID)
    echo -e "UAC Caller $i PID: ${GREEN}$CALLER_PID${NC} (port $((6001 + i)))"
    sleep 0.5
done

echo ""
echo -e "${BLUE}=== All processes started ===${NC}"
echo "Logs:"
echo "  UAS: /tmp/uas_answerer.log"
echo "  UAC 0-4: /tmp/uac_caller_{0..4}.log"
echo ""
echo "Waiting for test to complete (30 seconds)..."

# Wait for completion
sleep 30

# Capture final resource usage using simple arrays
UAS_RSS=0
UAS_THREADS=0
if ps -p $UAS_PID > /dev/null 2>&1; then
    UAS_RSS=$(ps -o rss= -p $UAS_PID 2>/dev/null | tr -d ' ' || echo "0")
    UAS_THREADS=$(ps -M -p $UAS_PID 2>/dev/null | tail -n +2 | wc -l | tr -d ' ' || echo "1")
fi

# UAC arrays
UAC_RSS=()
UAC_THREADS=()
for i in {0..4}; do
    pid=${CALLER_PIDS[$i]}
    if ps -p $pid > /dev/null 2>&1; then
        rss=$(ps -o rss= -p $pid 2>/dev/null | tr -d ' ' || echo "0")
        threads=$(ps -M -p $pid 2>/dev/null | tail -n +2 | wc -l | tr -d ' ' || echo "1")
        UAC_RSS+=($rss)
        UAC_THREADS+=($threads)
    else
        UAC_RSS+=(0)
        UAC_THREADS+=(0)
    fi
done

echo ""
echo -e "${BLUE}=== Test complete ===${NC}"
echo "Checking for errors..."
echo ""

# Check each log for errors
ERROR_COUNT=0
UAS_ERRORS=$(grep "ERROR" /tmp/uas_answerer.log 2>/dev/null | wc -l | tr -d ' ')
UAS_ERRORS=${UAS_ERRORS:-0}
if [ "$UAS_ERRORS" -eq 0 ]; then
    echo -e "UAS Errors: ${GREEN}0${NC}"
else
    echo -e "UAS Errors: ${RED}$UAS_ERRORS${NC}"
    ERROR_COUNT=$((ERROR_COUNT + UAS_ERRORS))
fi

for i in {0..4}; do
    UAC_ERRORS=$(grep "ERROR" /tmp/uac_caller_$i.log 2>/dev/null | wc -l | tr -d ' ')
    UAC_ERRORS=${UAC_ERRORS:-0}
    if [ "$UAC_ERRORS" -eq 0 ]; then
        echo -e "UAC $i Errors: ${GREEN}0${NC}"
    else
        echo -e "UAC $i Errors: ${RED}$UAC_ERRORS${NC}"
        ERROR_COUNT=$((ERROR_COUNT + UAC_ERRORS))
    fi
done

echo ""
echo -e "${BLUE}=== Resource Usage Report ===${NC}"
echo ""

# Function to format bytes
format_bytes() {
    local kb=$1
    local mb=$(awk "BEGIN {printf \"%.2f\", $kb/1024}")
    echo "${mb} MB"
}

# UAS Report
echo -e "${YELLOW}UAS Answerer:${NC}"
if [ "$UAS_RSS" != "0" ] && [ -n "$UAS_RSS" ]; then
    echo "  Memory (RSS): $(format_bytes $UAS_RSS)"
    echo "  Threads: $UAS_THREADS"
else
    echo "  Process completed"
fi
echo ""

# UAC Reports
TOTAL_RSS=$UAS_RSS
TOTAL_THREADS=$UAS_THREADS

for i in {0..4}; do
    echo -e "${YELLOW}UAC Caller $i:${NC}"
    rss=${UAC_RSS[$i]}
    threads=${UAC_THREADS[$i]}

    if [ "$rss" != "0" ] && [ -n "$rss" ]; then
        echo "  Memory (RSS): $(format_bytes $rss)"
        echo "  Threads: $threads"
        TOTAL_RSS=$((TOTAL_RSS + rss))
        TOTAL_THREADS=$((TOTAL_THREADS + threads))
    else
        echo "  Process completed"
    fi
    echo ""
done

echo -e "${BLUE}Total Resource Usage:${NC}"
echo "  Combined Memory: $(format_bytes $TOTAL_RSS)"
echo "  Combined Threads: $TOTAL_THREADS"
echo ""

# Summary
echo -e "${BLUE}=== Benchmark Summary ===${NC}"
echo "  Concurrent Calls: 5"
echo "  Total Processes: 6 (1 UAS + 5 UAC)"
if [ "$ERROR_COUNT" -eq 0 ]; then
    echo -e "  Total Errors: ${GREEN}0${NC} ✓"
else
    echo -e "  Total Errors: ${RED}$ERROR_COUNT${NC}"
fi
echo ""

echo -e "${BLUE}=== Sample output from UAS ===${NC}"
tail -20 /tmp/uas_answerer.log

# Cleanup any remaining processes
pkill -P $$