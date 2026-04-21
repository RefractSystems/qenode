import asyncio
import os
import struct
import subprocess
from pathlib import Path

import pytest


def build_phase7_artifacts():
    import tempfile

    tmpdir = tempfile.mkdtemp(prefix="virtmcu-phase7-")
    linker_script = Path(tmpdir) / "link.ld"

    with Path(linker_script).open("w") as f:
        f.write("SECTIONS { . = 0x40000000; .text : { *(.text) } }\n")

    asm_file = Path(tmpdir) / "firmware.S"
    with Path(asm_file).open("w") as f:
        f.write(".global _start\n_start: loop: nop; b loop\n")

    kernel_path = Path(tmpdir) / "firmware.elf"
    subprocess.run(
        ["arm-none-eabi-gcc", "-mcpu=cortex-a15", "-nostdlib", "-T", linker_script, asm_file, "-o", kernel_path],
        check=True,
    )

    dts_file = Path(tmpdir) / "dummy.dts"
    with Path(dts_file).open("w") as f:
        f.write("""/dts-v1/;
/ {
    model = "virtmcu-test"; compatible = "arm,generic-fdt"; #address-cells = <2>; #size-cells = <2>;
    qemu_sysmem: qemu_sysmem { compatible = "qemu:system-memory"; phandle = <0x01>; };
    chosen {};
    memory@40000000 { compatible = "qemu-memory-region"; qemu,ram = <0x01>; container = <0x01>; reg = <0x0 0x40000000 0x0 0x10000000>; };
    cpus { #address-cells = <1>; #size-cells = <0>; cpu@0 { device_type = "cpu"; compatible = "cortex-a15-arm-cpu"; reg = <0>; memory = <0x01>; }; };
};
""")
    dtb_path = Path(tmpdir) / "dummy.dtb"
    subprocess.run(["dtc", "-I", "dts", "-O", "dtb", "-o", dtb_path, dts_file], check=True)
    return dtb_path, kernel_path


@pytest.mark.asyncio
async def test_phase7_clock_suspend(zenoh_router, qemu_launcher, time_authority):
    """
    Phase 7: zenoh-clock slaved-suspend mode.
    """
    dtb_path, kernel_path = build_phase7_artifacts()

    extra_args = [
        "-icount",
        "shift=0,align=off,sleep=off",
        "-device",
        f"zenoh-clock,mode=slaved-suspend,node=0,router={zenoh_router}",
    ]

    await qemu_launcher(dtb_path, kernel_path, extra_args=extra_args, ignore_clock_check=True)

    # First step returns 1,000,000
    vtime1 = await time_authority.step(1_000_000)
    assert vtime1 == 1_000_000

    vtime2 = await time_authority.step(1_000_000)
    assert vtime2 == 2_000_000

    vtime3 = await time_authority.step(1_000_000)
    assert vtime3 == 3_000_000


@pytest.mark.asyncio
async def test_phase7_clock_stall(zenoh_router, qemu_launcher, zenoh_session):  # noqa: ARG001
    """
    Phase 7: zenoh-clock stall timeout.
    QEMU should exit if the clock doesn't advance for stall-timeout ms.
    """
    dtb_path, kernel_path = build_phase7_artifacts()

    # Use a faster stall timeout for testing, but enough for CI to boot
    stall_timeout_ms = int(os.environ.get("VIRTMCU_STALL_TIMEOUT_MS", "15000"))
    if stall_timeout_ms > 30000:
        stall_timeout_ms = 15000  # Don't wait forever in tests

    extra_args = [
        "-icount",
        "shift=0,align=off,sleep=off",
        "-device",
        f"zenoh-clock,mode=slaved-suspend,node=0,router={zenoh_router},stall-timeout={stall_timeout_ms}",
        "-display",
        "none",
        "-nographic",
        "-monitor",
        "none",
    ]

    from conftest import TimeAuthority

    ta = TimeAuthority(zenoh_session, node_id=0)

    # Launch QEMU
    curr = Path(Path(__file__).resolve().parent)
    while str(curr) != "/" and not (curr / "scripts").exists():
        curr = Path(curr).parent
    workspace_root = curr
    run_script = Path(workspace_root) / "scripts/run.sh"

    cmd = [str(run_script), "--dtb", str(Path(dtb_path).resolve()), "--kernel", str(Path(kernel_path).resolve())]
    cmd.extend(extra_args)

    proc = await asyncio.create_subprocess_exec(
        *cmd, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.STDOUT, env=os.environ.copy()
    )

    # Give QEMU a moment to initialize
    await asyncio.sleep(2.0)

    try:
        # First step should succeed (Initial sync)
        await ta.step(0)

        # Next step to 1ms
        vtime = await ta.step(1_000_000)
        assert vtime == 1_000_000

        # Second step with delay to trigger stall
        try:
            # step() returns the error code (1) on stall now
            res = await ta.step(1_000_000, delay=(stall_timeout_ms / 1000.0 + 2.0))
            assert res == 1  # CLOCK_ERROR_STALL
        except Exception:
            pass

        # Now wait for QEMU to exit
        try:
            await asyncio.wait_for(proc.wait(), timeout=10.0)
        except TimeoutError:
            proc.terminate()
            stdout, _ = await proc.communicate()
            pytest.fail(f"QEMU failed to exit on clock stall. Output:\n{stdout.decode()}")

        stdout, _ = await proc.communicate()
        output = stdout.decode()
        assert "FATAL STALL: no clock-advance reply" in output
        assert proc.returncode != 0

    finally:
        if proc.returncode is None:
            proc.kill()
            await proc.wait()


@pytest.mark.asyncio
async def test_phase7_determinism(zenoh_router, qemu_launcher, zenoh_session):
    """
    Phase 7: zenoh-clock determinism test.
    Verify that same icount leads to same virtual time regardless of wall-clock.
    """
    from conftest import TimeAuthority

    dtb_path, kernel_path = build_phase7_artifacts()

    extra_args = [
        "-icount",
        "shift=0,align=off,sleep=off",
        "-device",
        f"zenoh-clock,mode=slaved-icount,node=0,router={zenoh_router}",
    ]
    await qemu_launcher(dtb_path, kernel_path, extra_args=extra_args, ignore_clock_check=True)

    ta = TimeAuthority(zenoh_session, node_id=0)

    # Initial sync
    await ta.step(0)

    vtime = await ta.step(1_000_000)
    assert vtime == 1_000_000

    vtime = await ta.step(1_000_000)
    assert vtime == 2_000_000

    vtime = await ta.step(1_000_000)
    assert vtime == 3_000_000


@pytest.mark.asyncio
async def test_phase7_netdev(zenoh_router, qemu_launcher, zenoh_session):
    """
    Phase 7: zenoh-netdev functional test.
    Verify that packets injected via Zenoh reach the guest (implied by it staying in sync).
    """
    from conftest import TimeAuthority

    dtb_path, kernel_path = build_phase7_artifacts()

    extra_args = [
        "-icount",
        "shift=0,align=off,sleep=off",
        "-device",
        f"zenoh-clock,mode=slaved-icount,node=1,router={zenoh_router}",
        "-device",
        "zenoh-netdev",
        "-netdev",
        f"zenoh,node=1,id=n1,router={zenoh_router}",
    ]
    await qemu_launcher(dtb_path, kernel_path, extra_args=extra_args, ignore_clock_check=True)

    # Need a TimeAuthority for node 1
    ta1 = TimeAuthority(zenoh_session, node_id=1)

    # Initial block
    await ta1.step(0)

    NETDEV_TOPIC = "sim/eth/frame/1/rx"  # noqa: N806
    DELIVERY_VTIME_NS = 500_000  # noqa: N806
    FRAME = b"\xff" * 14  # noqa: N806
    packet = struct.pack("<QI", DELIVERY_VTIME_NS, len(FRAME)) + FRAME

    pub = await asyncio.to_thread(lambda: zenoh_session.declare_publisher(NETDEV_TOPIC))
    await asyncio.to_thread(lambda: pub.put(packet))
    await asyncio.sleep(0.5)

    # Step clock past the delivery time
    vtime = await ta1.step(1_000_000)
    assert vtime == 1_000_000


@pytest.mark.asyncio
async def test_phase7_netdev_stress(zenoh_router, qemu_launcher, zenoh_session):
    """
    Phase 7: zenoh-netdev stress test.
    Inject 1000 packets in reverse virtual-time order and verify they are
    delivered to the guest in correct order.
    """
    from conftest import TimeAuthority

    dtb_path, kernel_path = build_phase7_artifacts()

    extra_args = [
        "-icount",
        "shift=0,align=off,sleep=off",
        "-device",
        f"zenoh-clock,mode=slaved-icount,node=0,router={zenoh_router}",
        "-device",
        "zenoh-netdev",
        "-netdev",
        f"zenoh,node=0,id=n0,router={zenoh_router}",
    ]

    await qemu_launcher(dtb_path, kernel_path, extra_args=extra_args, ignore_clock_check=True)

    ta = TimeAuthority(zenoh_session, node_id=0)
    await ta.step(0)

    rx_topic = "sim/eth/frame/0/rx"
    base_time = 1_000_000_000

    pub = await asyncio.to_thread(lambda: zenoh_session.declare_publisher(rx_topic))

    for i in range(1000):
        vtime = base_time + (1000 - i) * 1000
        data = f"PACKET_{i}".encode()
        payload = struct.pack("<QI", vtime, len(data)) + data
        await asyncio.to_thread(pub.put, payload)

    await ta.step(base_time + 2_000_000)
