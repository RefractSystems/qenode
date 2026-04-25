#!/bin/bash
# test/phase14/smoke_test.sh
# Verifies that Phase 14 wireless devices are correctly parsed and emitted.

set -euo pipefail

echo "Testing Phase 14: Wireless & IoT RF Simulation..."

# 1. Run yaml2qemu to generate CLI and DTB
python3 -m tools.yaml2qemu test/phase14/board.yaml --out-dtb test/phase14/test.dtb --out-cli test/phase14/test.cli

# 2. Verify CLI arguments
echo "Verifying CLI arguments..."
grep -q "zenoh-802154,node=0" test/phase14/test.cli
grep -q "zenoh,id=hci0,node=0,topic=sim/rf/hci/0" test/phase14/test.cli
echo "✓ CLI arguments correct."

# 3. Verify DTB contains radio0
echo "Verifying DTB..."
dtc -I dtb -O dts test/phase14/test.dtb | grep -q "radio0 {"
echo "✓ DTB contains radio0 node."

# 4. Check coordinator build
echo "Verifying coordinator..."
COORDINATOR_BIN=""
if command -v zenoh_coordinator >/dev/null 2>&1; then
    COORDINATOR_BIN=$(command -v zenoh_coordinator)
    echo "Using pre-built coordinator from PATH."
elif [ -f "target/release/zenoh_coordinator" ]; then
    COORDINATOR_BIN="target/release/zenoh_coordinator"
    echo "Using workspace release coordinator."
elif [ -f "target/debug/zenoh_coordinator" ]; then
    COORDINATOR_BIN="target/debug/zenoh_coordinator"
    echo "Using workspace debug coordinator."
elif [ -f "tools/zenoh_coordinator/target/release/zenoh_coordinator" ]; then
    COORDINATOR_BIN="tools/zenoh_coordinator/target/release/zenoh_coordinator"
    echo "Using tool-specific release coordinator."
else
    # Subshell to avoid changing the CWD of this script.
    ( cd tools/zenoh_coordinator && cargo build -q --release )
    COORDINATOR_BIN="tools/zenoh_coordinator/target/release/zenoh_coordinator"
fi
# Verify the binary is executable and responds to --help.
"$COORDINATOR_BIN" --help >/dev/null 2>&1 || "$COORDINATOR_BIN" --version >/dev/null 2>&1 || {
    echo "ERROR: zenoh_coordinator at $COORDINATOR_BIN failed to execute." >&2
    exit 1
}
echo "✓ Coordinator is present and executable."


echo "Phase 14 Smoke Test PASSED."
