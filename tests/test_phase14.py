import subprocess
from pathlib import Path


def test_phase14_parsing():
    """
    Phase 14: Wireless & IoT RF Simulation.
    Verify that wireless devices are correctly parsed and emitted.
    """
    workspace_root = Path(__file__).resolve().parent.parent
    yaml_file = workspace_root / "test/phase14/board.yaml"
    dtb_out = workspace_root / "test/phase14/test.dtb"
    cli_out = workspace_root / "test/phase14/test.cli"

    subprocess.run(
        ["python3", "-m", "tools.yaml2qemu", str(yaml_file), "--out-dtb", str(dtb_out), "--out-cli", str(cli_out)],
        check=True,
        cwd=workspace_root,
    )

    cli_content = cli_out.read_text()
    assert "zenoh-802154,node=0" in cli_content
    assert "zenoh,id=hci0,node=0,topic=sim/rf/hci/0" in cli_content

    dtc_output = subprocess.check_output(["dtc", "-I", "dtb", "-O", "dts", str(dtb_out)], text=True)
    assert "radio0@9001000" in dtc_output
