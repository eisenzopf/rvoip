#!/bin/bash

# Test script for sip-client-p2p with full debug logging

echo "Starting SIP P2P test with debug logging..."
echo "Run this in receiver mode in one terminal and caller mode in another."
echo ""

if [ "$1" == "receive" ]; then
    echo "Starting receiver on port ${3:-5060}..."
    RUST_LOG=rvoip_sip_client=debug,rvoip_audio_core=debug,sip_client_p2p=info ./target/release/sip-client-p2p receive -n "${2:-alice}" -p "${3:-5060}"
elif [ "$1" == "call" ]; then
    echo "Starting caller to $2:${3:-5060}..."
    RUST_LOG=rvoip_sip_client=debug,rvoip_audio_core=debug,sip_client_p2p=info ./target/release/sip-client-p2p call -n "${4:-bob}" -t "$2" -P "${3:-5060}" -p "${5:-5061}"
else
    echo "Usage:"
    echo "  Receiver: $0 receive [name] [port]"
    echo "  Caller:   $0 call <target_ip> [target_port] [name] [local_port]"
    echo ""
    echo "Examples:"
    echo "  $0 receive alice 5060"
    echo "  $0 call 192.168.1.100 5060 bob 5061"
fi