"""
Wireless & IoT RF Simulation.
Verify that wireless devices are correctly parsed and emitted.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from pathlib import Path


def test_parsing(tmp_path: Path, guest_app_factory: Any) -> None:  # noqa: ANN401
    from tools.testing.virtmcu_test_suite.factory import compile_yaml, inspect_dtb

    app_dir = guest_app_factory("complex_board")
    yaml_file = app_dir / "board.yaml"
    dtb_out = tmp_path / "test.dtb"
    cli_out = tmp_path / "test.cli"

    compile_yaml(yaml_file, dtb_out, out_cli=cli_out)

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

    dtc_output = inspect_dtb(dtb_out)
    assert "uart0@9000000 {" in dtc_output
    assert "radio0@9001000 {" in dtc_output

    assert 'transport = "zenoh";' in dtc_output
