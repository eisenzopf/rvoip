#!/bin/bash

# Thread benchmark runner script
# Tests thread usage with 1 answering peer and 5 calling peers

echo "🧵 Thread Benchmark: 1 Answerer + 5 Callers"
echo "=========================================="
echo ""

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

# Set the state table environment variable
export RVOIP_STATE_TABLE="/Users/jonathan/Documents/Work/Rudeless_Ventures/rvoip/crates/session-core-v2/examples/api_peer_audio/peer_audio_states.yaml"

# Parse arguments
PROFILE="release"
PROFILING=0
LOG_LEVEL="info"

for arg in "$@"; do
    case $arg in
        --debug)
            PROFILE="dev"
            echo -e "${YELLOW}🔧 Using debug build${NC}"
            ;;
        --flamegraph)
            PROFILE="flamegraph"
            echo -e "${YELLOW}🔥 Using flamegraph profile (with debug symbols)${NC}"
            ;;
        --profile)
            PROFILING=1
            PROFILE="flamegraph"
            echo -e "${YELLOW}📊 Profiling enabled with Instruments${NC}"
            ;;
        --trace)
            LOG_LEVEL="debug"
            echo -e "${YELLOW}🔍 Trace logging enabled${NC}"
            ;;
        --help)
            echo "Usage: $0 [options]"
            echo "Options:"
            echo "  --debug       Use debug build instead of release"
            echo "  --flamegraph  Use flamegraph profile (optimized with debug symbols)"
            echo "  --profile     Run with Instruments Time Profiler"
            echo "  --trace       Enable debug-level logging"
            echo "  --help        Show this help message"
            exit 0
            ;;
    esac
done

# Set logging level
export RUST_LOG="rvoip_session_core_v2=$LOG_LEVEL,rvoip_dialog_core=$LOG_LEVEL,rvoip_media_core=$LOG_LEVEL"

# Navigate to the project root
cd "$(dirname "$0")/../../../.."

# Build the benchmark
echo -e "${GREEN}🔨 Building thread_benchmark with $PROFILE profile...${NC}"
if [ "$PROFILE" = "dev" ]; then
    cargo build --example thread_benchmark -p rvoip-session-core-v2
    BINARY="./target/debug/examples/thread_benchmark"
elif [ "$PROFILE" = "flamegraph" ]; then
    cargo build --profile flamegraph --example thread_benchmark -p rvoip-session-core-v2
    BINARY="./target/flamegraph/examples/thread_benchmark"
else
    cargo build --release --example thread_benchmark -p rvoip-session-core-v2
    BINARY="./target/release/examples/thread_benchmark"
fi

if [ $? -ne 0 ]; then
    echo -e "${RED}❌ Build failed${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Build successful${NC}"
echo ""

# Run with profiling if requested
if [ $PROFILING -eq 1 ]; then
    echo -e "${BLUE}📊 Starting profiling with Instruments...${NC}"
    echo -e "${YELLOW}Press Ctrl+C to stop profiling when the benchmark completes${NC}"

    TIMESTAMP=$(date +%Y%m%d_%H%M%S)
    TRACE_FILE="thread_benchmark_${TIMESTAMP}.trace"

    xcrun xctrace record --template "Time Profiler" --output "$TRACE_FILE" --launch -- "$BINARY"

    echo ""
    echo -e "${GREEN}✅ Profiling complete. Trace saved to: $TRACE_FILE${NC}"
    echo -e "${BLUE}Opening trace in Instruments...${NC}"
    open "$TRACE_FILE"
else
    # Regular run
    echo -e "${BLUE}🚀 Starting benchmark...${NC}"
    echo ""

    # Run the benchmark
    "$BINARY"

    EXIT_CODE=$?

    echo ""
    if [ $EXIT_CODE -eq 0 ]; then
        echo -e "${GREEN}✅ Benchmark completed successfully${NC}"
    else
        echo -e "${RED}❌ Benchmark failed with exit code $EXIT_CODE${NC}"
        exit $EXIT_CODE
    fi
fi

echo ""
echo "📊 Benchmark Summary:"
echo "  - 1 answering peer on port 6000"
echo "  - 5 calling peers on ports 6001-6005"
echo "  - 6 total concurrent SIP sessions"
echo ""
echo "Check the output above for thread count and resource usage metrics"