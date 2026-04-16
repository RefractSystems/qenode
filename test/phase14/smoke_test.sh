#!/bin/bash
# test/phase14/smoke_test.sh
# Verifies that Phase 14 wireless devices are correctly parsed and emitted.

set -e

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
dtc -I dtb -O dts test/phase14/test.dtb | grep -q "radio0@9001000"
echo "✓ DTB contains radio0 node."

# 4. Check coordinator build
echo "Verifying coordinator..."
cd tools/zenoh_coordinator && cargo build -q
echo "✓ Coordinator builds successfully."

echo "Phase 14 Smoke Test PASSED."
