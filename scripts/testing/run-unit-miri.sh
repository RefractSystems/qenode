#!/usr/bin/env bash
# Shim for native virtmcu-test-runner
set -euo pipefail
cargo run -p virtmcu-test-runner --release -- miri
