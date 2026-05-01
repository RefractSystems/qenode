#!/bin/bash
# tests/fixtures/guest_apps/complex_board/smoke_test.sh
# Verifies that wireless devices are correctly parsed and emitted.

set -euo pipefail

echo "Testing Wireless & IoT RF Simulation..."

# 1. Run yaml2qemu to generate CLI and DTB
python3 -m tools.yaml2qemu tests/fixtures/guest_apps/complex_board/board.yaml --out-dtb tests/fixtures/guest_apps/complex_board/test.dtb --out-cli tests/fixtures/guest_apps/complex_board/test.cli

# 2. Verify CLI arguments
echo "Verifying CLI arguments..."

# Extract the device lines to check properties independently of order
# NOTE: ieee802154 is now handled via DTB injection, not CLI flags.

HCI_CLI=$(grep "^virtmcu" tests/fixtures/guest_apps/complex_board/test.cli || true)
if [ -z "$HCI_CLI" ] || ! echo "$HCI_CLI" | grep -q "id=hci0" || ! echo "$HCI_CLI" | grep -q "node=0" || ! echo "$HCI_CLI" | grep -q "transport=zenoh" || ! echo "$HCI_CLI" | grep -q "topic=sim/rf/hci/0"; then
    echo "ERROR: Missing or malformed virtmcu chardev in CLI: $HCI_CLI"
    exit 1
fi
echo "✓ CLI arguments correct."

# 3. Verify DTB contains radio0 and its properties
echo "Verifying DTB..."
DTS_CONTENT=$(dtc -I dtb -O dts tests/fixtures/guest_apps/complex_board/test.dtb)
if ! echo "$DTS_CONTENT" | grep -q "radio0@"; then
    echo "ERROR: DTB missing radio0 node."
    exit 1
fi

# Check ieee802154 properties in DTB
RADIO_NODE=$(echo "$DTS_CONTENT" | sed -n '/radio0@/,/};/p')
if ! echo "$RADIO_NODE" | grep -q 'compatible = "ieee802154"'; then
    echo "ERROR: radio0 missing compatible property in DTB"
    exit 1
fi
if ! echo "$RADIO_NODE" | grep -q 'node = <0x00>'; then
    echo "ERROR: radio0 missing or wrong node property in DTB"
    exit 1
fi
if ! echo "$RADIO_NODE" | grep -q 'transport = "zenoh"'; then
    echo "ERROR: radio0 missing or wrong transport property in DTB"
    exit 1
fi

echo "✓ DTB contains radio0 node with correct properties."

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


echo "Smoke Test PASSED."
