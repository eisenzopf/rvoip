#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../../../.."

cargo run -p rvoip-session-core --example freeswitch_registration
