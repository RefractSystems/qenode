import subprocess


def test_parsing(tmp_path):
    """
    Wireless & IoT RF Simulation.
    Verify that wireless devices are correctly parsed and emitted.
    """
    from tools.testing.env import WORKSPACE_ROOT

    workspace_root = WORKSPACE_ROOT
    yaml_file = workspace_root / "tests/fixtures/guest_apps/complex_board/board.yaml"
    dtb_out = tmp_path / "test.dtb"
    cli_out = tmp_path / "test.cli"

    subprocess.run(
        ["python3", "-m", "tools.yaml2qemu", str(yaml_file), "--out-dtb", str(dtb_out), "--out-cli", str(cli_out)],
        check=True,
        cwd=workspace_root,
    )

    cli_lines = cli_out.read_text().splitlines()

    # Robust order-independent verification for the chardev
    assert any(
        line.startswith("virtmcu")
        and "id=hci0" in line.split(",")
        and "node=0" in line.split(",")
        and "transport=zenoh" in line.split(",")
        and "topic=sim/rf/hci/0" in line.split(",")
        for line in cli_lines
    ), f"Could not find valid virtmcu chardev configuration in: {cli_lines}"

    dtc_output = subprocess.check_output(["dtc", "-I", "dtb", "-O", "dts", str(dtb_out)], text=True)
    assert "uart0@9000000 {" in dtc_output
    assert "radio0@9001000 {" in dtc_output

    assert 'transport = "zenoh";' in dtc_output
