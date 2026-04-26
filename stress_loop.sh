#!/bin/bash
set -euo pipefail

ARCH=$(uname -m | sed -e "s/x86_64/amd64/" -e "s/aarch64/arm64/")
BUILDER_IMG="ghcr.io/refractsystems/virtmcu/builder:dev-${ARCH}"

echo "==> Ensuring builder image exists for ${ARCH}..."
bash scripts/docker-build.sh builder

docker run --rm \
    --user root \
    -e HOME=/tmp \
    -v "$(pwd):/workspace" -w /workspace \
    "${BUILDER_IMG}" \
    bash -c "cp -r /workspace/hw/rust/* /build/virtmcu-hw/rust/ && cd /build/qemu/build-virtmcu && ninja hw-virtmcu-mmio-socket-bridge.so && cp hw-virtmcu-mmio-socket-bridge.so /opt/virtmcu/lib/qemu/ && cd /workspace && uv pip install --link-mode=copy --system --break-system-packages -r pyproject.toml >/dev/null && for i in {1..20}; do echo '===== RUN '\$i' ====='; PYTHONPATH=/workspace pytest tests/test_flexray.py::test_flexray_stress -s || exit 1; done"
echo "All 20 iterations passed!"

