import os

import pytest

from tools.testing.virtmcu_test_suite.process import AsyncManagedProcess


@pytest.mark.asyncio
async def test_canfd_plugin_loads():
    env = os.environ.copy()

    # We must run it via run.sh to get module paths right
    cmd = [
        "bash",
        "scripts/run.sh",
        "--dtb",
        "test/phase1/minimal.dtb",
        "-object",
        "can-bus,id=canbus0",
        "-object",
        "can-host-virtmcu,id=canhost0,canbus=canbus0,node=test_node,router=,topic=sim/can",
        "-monitor",
        "none",
        "-serial",
        "none",
        "-nographic",
        "-display",
        "none",
        "-S",
    ]

    async with AsyncManagedProcess(*cmd, env=env) as proc:
        try:
            await proc.wait(timeout=2.0)
        except TimeoutError:
            # QEMU shouldn't exit if it's running successfully with -S
            assert True
        else:
            print(f"STDOUT: {proc.stdout_text}")
            print(f"STDERR: {proc.stderr_text}")
            assert proc.returncode == 0, f"QEMU crashed or failed to load the plugin. STDERR: {proc.stderr_text}"
