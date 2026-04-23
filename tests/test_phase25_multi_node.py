import asyncio
import subprocess
import sys
import time
from pathlib import Path

import pytest

# Add tools/lin_fbs to sys.path
sys.path.append(str(Path.cwd() / "tools/lin_fbs"))

from virtmcu.lin import LinFrame, LinMessageType


@pytest.mark.asyncio
async def test_multi_node_lin(zenoh_router, qemu_launcher, zenoh_coordinator, zenoh_session):  # noqa: ARG001
    import shutil
    import tempfile

    tmpdir = tempfile.mkdtemp(prefix="virtmcu-lin-multi-")

    router_endpoint = zenoh_router

    # 1. Build Master/Slave ELFs
    master_kernel = Path(tmpdir) / "lin_master.elf"
    slave_kernel = Path(tmpdir) / "lin_slave.elf"

    subprocess.run(
        f"arm-none-eabi-gcc -mcpu=cortex-a15 -nostdlib -T test/phase25/lin_echo.ld test/phase25/lin_master.S -o {master_kernel}",
        shell=True,
        check=True,
    )
    subprocess.run(
        f"arm-none-eabi-gcc -mcpu=cortex-a15 -nostdlib -T test/phase25/lin_echo.ld test/phase25/lin_slave.S -o {slave_kernel}",
        shell=True,
        check=True,
    )

    # Use unique topic to avoid interference
    import uuid

    unique_id = str(uuid.uuid4())[:8]
    lin_topic = f"sim/lin/{unique_id}"

    # Generate Master DTB in tmpdir
    master_dtb = Path(tmpdir) / "lin_master.dtb"  # Replace router and compile
    subprocess.run(
        f"sed -e 's|tcp/127.0.0.1:7447|{router_endpoint}|' -e 's|\"sim/lin\"|\"{lin_topic}\"|' test/phase25/lin_test.dts | dtc -I dts -O dtb -o {master_dtb}",
        shell=True,
        check=True,
    )

    # Master node (Node 0)
    master_args = [
        "-cpu",
        "cortex-a15",
        "-chardev",
        "null,id=n0",
        "-serial",
        "chardev:n0",
        "-icount",
        "shift=0,align=off,sleep=off",
        "-net",
        "none",
        "-device",
        f"zenoh-clock,mode=slaved-icount,node=0,router={router_endpoint},stall-timeout=30000",
    ]

    # Generate Slave DTB
    slave_dtb = Path(tmpdir) / "lin_slave.dtb"
    subprocess.run(
        f"sed -e 's/node = <0>;/node = <1>;/' -e 's|tcp/127.0.0.1:7447|{router_endpoint}|' -e 's|\"sim/lin\"|\"{lin_topic}\"|' test/phase25/lin_test.dts | dtc -I dts -O dtb -o {slave_dtb}",
        shell=True,
        check=True,
    )
    slave_args = [
        "-cpu",
        "cortex-a15",
        "-chardev",
        "null,id=n1",
        "-serial",
        "chardev:n1",
        "-icount",
        "shift=0,align=off,sleep=off",
        "-net",
        "none",
        "-device",
        f"zenoh-clock,mode=slaved-icount,node=1,router={router_endpoint},stall-timeout=30000",
    ]

    print("Launching Master...")
    await qemu_launcher(master_dtb, master_kernel, extra_args=master_args, ignore_clock_check=True)
    print("Launching Slave...")
    await qemu_launcher(slave_dtb, slave_kernel, extra_args=slave_args, ignore_clock_check=True)

    # 3. Connect to Zenoh
    session = zenoh_session

    bus_messages = []

    def on_bus_msg(sample):
        try:
            payload = sample.payload.to_bytes()
            frame = LinFrame.LinFrame.GetRootAsLinFrame(payload, 0)
            msg_type = frame.Type()
            data_len = frame.DataLength()
            data = bytes([frame.Data(i) for i in range(data_len)])
            topic = str(sample.key_expr)
            print(f"Bus: {topic} type={msg_type} data={data!r}")
            bus_messages.append((topic, msg_type, data))
        except Exception as e:
            print(f"Error: {e}")

    # Listen to both nodes' TX
    sub0 = await asyncio.to_thread(lambda: session.declare_subscriber(f"{lin_topic}/0/tx", on_bus_msg))
    sub1 = await asyncio.to_thread(lambda: session.declare_subscriber(f"{lin_topic}/1/tx", on_bus_msg))

    from conftest import TimeAuthority

    ta0 = TimeAuthority(session, node_id=0)
    ta1 = TimeAuthority(session, node_id=1)

    try:
        # Wait for interaction
        timeout = 20
        start_time = time.time()
        found_master_header = False
        found_slave_response = False

        # Clock advance loop
        current_vtime = 0
        step_ns = 1_000_000  # 1ms

        while time.time() - start_time < timeout:
            # Advance both nodes
            await asyncio.gather(ta0.step(step_ns), ta1.step(step_ns))
            current_vtime += step_ns

            for topic, msg_type, data in bus_messages:
                if topic == f"{lin_topic}/0/tx" and msg_type == LinMessageType.LinMessageType.Break:
                    found_master_header = True
                if topic == f"{lin_topic}/1/tx" and msg_type == LinMessageType.LinMessageType.Data and b"S" in data:
                    found_slave_response = True

            if found_master_header and found_slave_response:
                break

            await asyncio.sleep(0.01)

        if found_master_header and found_slave_response:
            print(f"SUCCESS: Multi-node LIN communication verified at vtime={current_vtime}!")
        else:
            pytest.fail(
                f"FAILED: found_master_header={found_master_header}, found_slave_response={found_slave_response}, vtime={current_vtime}"
            )

    finally:
        await asyncio.to_thread(sub0.undeclare)
        await asyncio.to_thread(sub1.undeclare)
        shutil.rmtree(tmpdir)
