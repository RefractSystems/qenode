"""
SOTA Test Module: test_coverage_gap

Context:
This module implements tests for the test_coverage_gap subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_coverage_gap.
"""

import sys
from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest
import yaml

# Ensure we can import from tools
sys.path.insert(0, str(Path(__file__).resolve().parent / ".."))

from tools.qmp_probe import QMPClient, dump_tree
from tools.qmp_probe import main as qmp_main
from tools.repl2qemu.__main__ import main as repl2qemu_main
from tools.repl2qemu.fdt_emitter import FdtEmitter
from tools.repl2qemu.parser import ReplDevice, ReplPlatform, parse_repl
from tools.repl2yaml import main as repl2yaml_main
from tools.repl2yaml import migrate
from tools.usd_to_virtmcu import parse_yaml as usd_parse_yaml
from tools.yaml2qemu import main, parse_yaml_platform, validate_dtb


def test_repl2qemu_main_basic(tmp_path):
    repl_content = "cpu: CPU.ARMv7A @ sysbus"
    repl_file = tmp_path / "test.repl"
    repl_file.write_text(repl_content)
    dtb_file = tmp_path / "test.dtb"
    arch_file = tmp_path / "test.arch"

    test_args = [
        "repl2qemu.py",
        str(repl_file),
        "--out-dtb",
        str(dtb_file),
        "--out-arch",
        str(arch_file),
        "--print-cmd",
    ]

    with patch("sys.argv", test_args), patch("tools.repl2qemu.__main__.compile_dtb", return_value=True):
        repl2qemu_main()

    assert Path(arch_file).exists()


def test_repl2qemu_parser_edge_cases():
    repl_content = """
usart1: UART.STM32_UART @ sysbus {
    address: 0x40011000
}

[0-3] -> gic@[19-22] // this should be ignored if not in device block

device2: Some.Type @ sysbus
    [0-1] -> nvic@[10-11]
    some_prop: "quoted string"
"""
    platform = parse_repl(repl_content)
    assert len(platform.devices) == 2
    dev1 = platform.devices[0]
    assert dev1.address_str == "0x40011000"

    dev2 = platform.devices[1]
    assert len(dev2.interrupts) == 1
    assert dev2.interrupts[0].source_range == "0-1"
    assert dev2.properties["some_prop"] == "quoted string"


def test_repl2yaml_migrate(tmp_path):
    repl_content = """
cpu: CPU.ARMv7A @ sysbus
    cpuType: "cortex-a15"

uart0: UART.PL011 @ sysbus 0x09000000
    interrupts: 37
"""
    repl_file = tmp_path / "test.repl"
    repl_file.write_text(repl_content)
    yaml_file = tmp_path / "test.yaml"

    migrate(str(repl_file), str(yaml_file))

    assert Path(yaml_file).exists()
    with Path(yaml_file).open() as f:
        data = yaml.safe_load(f)
        assert data["machine"]["cpus"][0]["name"] == "cpu"
        assert data["peripherals"][0]["name"] == "uart0"
        assert data["peripherals"][0]["address"] == "0x09000000"


def test_repl2yaml_main(tmp_path):
    repl_file = tmp_path / "test_main.repl"
    repl_file.write_text("cpu: CPU.ARMv7A @ sysbus")

    test_args = ["repl2yaml.py", str(repl_file)]
    with patch("sys.argv", test_args), patch("tools.repl2yaml.migrate") as mock_migrate:
        repl2yaml_main()
        mock_migrate.assert_called_with(str(repl_file), "test_main.yaml")


def test_usd_to_virtmcu(tmp_path, capsys):
    yaml_content = """
peripherals:
  - name: uart-0
    address: 0x9000000
"""
    yaml_file = tmp_path / "test.yaml"
    yaml_file.write_text(yaml_content)

    usd_parse_yaml(str(yaml_file))

    captured = capsys.readouterr()
    assert "constexpr uint64_t UART_0_BASE = 150994944;" in captured.out


def test_qmp_client_basic():
    with patch("socket.socket") as mock_socket:
        mock_sock_inst = mock_socket.return_value
        # Mock greeting
        mock_sock_inst.recv.side_effect = [
            b'{"QMP": {"version": {"qemu": {"micro": 0, "minor": 0, "major": 9}, "package": ""}, "capabilities": []}}\n',
            b'{"return": {}}\n',  # response to qmp_capabilities
        ]

        client = QMPClient("dummy.sock")
        client.connect()

        assert mock_sock_inst.connect.called
        assert mock_sock_inst.send.called


def test_qmp_dump_tree(caplog):
    client = MagicMock()
    # Mock qom-list response
    client.execute.side_effect = [
        {"return": [{"name": "machine", "type": "child<container>"}, {"name": "type", "type": "string"}]},
        {"return": [{"name": "unattached", "type": "child<container>"}]},
        {"return": []},
    ]

    dump_tree(client, path="/")

    assert "machine (child<container>)" in caplog.text
    assert "unattached (child<container>)" in caplog.text


def test_qmp_main_tree():
    with patch("tools.qmp_probe.QMPClient") as mock_client_cls, patch("tools.qmp_probe.dump_tree") as mock_dump_tree:
        test_args = ["qmp_probe.py", "--socket", "test.sock", "tree"]
        with patch("sys.argv", test_args):
            qmp_main()

        mock_client_cls.assert_called_with("test.sock")
        assert mock_dump_tree.called


def test_qmp_main_list(caplog):
    with patch("tools.qmp_probe.QMPClient") as mock_client_cls:
        mock_client = mock_client_cls.return_value
        mock_client.execute.return_value = {
            "return": [{"name": "dev0", "type": "child<device>"}, {"name": "prop1", "type": "string"}]
        }

        test_args = ["qmp_probe.py", "list", "/machine"]
        with patch("sys.argv", test_args):
            qmp_main()


        assert "dev0" in caplog.text
        assert "prop1" in caplog.text


def test_qmp_main_get(caplog):
    with patch("tools.qmp_probe.QMPClient") as mock_client_cls:
        mock_client = mock_client_cls.return_value
        mock_client.execute.return_value = {"return": 1234}

        test_args = ["qmp_probe.py", "get", "/path", "prop"]
        with patch("sys.argv", test_args):
            qmp_main()


        assert "1234" in caplog.text


def test_fdt_emitter_64bit_memory():
    platform = ReplPlatform()
    platform.devices.append(
        ReplDevice(
            name="highmem",
            type_name="Memory.MappedMemory",
            address_str="0x100000000",
            properties={"size": "0x100000000"},
        )
    )
    emitter = FdtEmitter(platform)
    dts = emitter.generate_dts()
    assert "memory@100000000" in dts
    assert "reg = <0x1 0x0 0x1 0x0>;" in dts


def test_qmp_client_connect_fail():
    with patch("socket.socket") as mock_socket:
        mock_sock_inst = mock_socket.return_value
        mock_sock_inst.connect.side_effect = FileNotFoundError

        client = QMPClient("nonexistent.sock")
        with pytest.raises(SystemExit) as e:
            client.connect()
        assert e.value.code == 1


def test_qmp_client_recv_empty():
    with patch("socket.socket") as mock_socket:
        mock_sock_inst = mock_socket.return_value
        # Greeting, qmp_capabilities response, then empty for test call
        mock_sock_inst.recv.side_effect = [b'{"QMP": {}}\n', b'{"return": {}}\n', b""]

        client = QMPClient("dummy.sock")
        client.connect()
        assert client._recv_msg() is None


def test_fdt_emitter_riscv():
    platform = ReplPlatform()
    platform.devices.append(
        ReplDevice(
            name="cpu0",
            type_name="CPU.RISCV64",
            address_str="sysbus",
            properties={"cpuType": "rv64", "isa": "rv64imac", "mmu-type": "riscv,sv39"},
        )
    )
    emitter = FdtEmitter(platform)
    assert emitter.arch == "riscv"
    dts = emitter.generate_dts()
    assert 'compatible = "riscv-virtio";' in dts
    assert 'riscv,isa = "rv64imac";' in dts
    assert 'mmu-type = "riscv,sv39";' in dts
    assert "interrupt-controller" in dts
    assert "timebase-frequency" in dts


def test_fdt_emitter_parse_addr_edge_cases():
    emitter = FdtEmitter(ReplPlatform())
    # Test none
    assert emitter._parse_addr("none") == (0, 0)
    assert emitter._parse_addr("") == (0, 0)
    # Test invalid hex
    assert emitter._parse_addr("invalid") == (0, 0)


def test_fdt_emitter_extra_properties():
    platform = ReplPlatform()
    dev = ReplDevice(
        name="testdev",
        type_name="UART.PL011",
        address_str="0x1000",
        properties={"bool_prop": True, "int_prop": 123, "str_prop": "hello", "ignored_prop": "size"},
    )
    platform.devices.append(dev)
    emitter = FdtEmitter(platform)
    dts = emitter.generate_dts()
    assert "bool_prop;" in dts
    assert "int_prop = <0x7b>;" in dts
    assert 'str_prop = "hello";' in dts


def test_yaml2qemu_riscv_mapping(tmp_path):
    yaml_content = """
machine:
  cpus:
    - name: cpu0
      type: riscv-rv64
      isa: rv64gc
      mmu-type: riscv,sv48
peripherals: []
"""
    yaml_file = tmp_path / "test_riscv.yaml"
    yaml_file.write_text(yaml_content)

    platform, _ = parse_yaml_platform(str(yaml_file))
    cpu = platform.devices[0]
    assert cpu.type_name == "CPU.RISCV64"
    assert cpu.properties["isa"] == "rv64gc"
    assert cpu.properties["mmu-type"] == "riscv,sv48"


def test_yaml2qemu_validate_dtb_failure(caplog):
    devices = [ReplDevice(name="uart0", type_name="UART.PL011", address_str="0x9000000", properties={})]

    with patch("subprocess.run") as mock_run:
        # Mock dtc output missing the device
        mock_run.return_value = MagicMock(stdout="nothing here", returncode=0)

        with pytest.raises(SystemExit) as e:
            validate_dtb("dummy.dtb", devices)
        assert e.value.code == 1


    assert "ERROR: The following peripherals from YAML are missing in the generated DTB: uart0" in caplog.text


def test_yaml2qemu_validate_dtb_dtc_missing(caplog):
    devices = [ReplDevice(name="uart0", type_name="UART.PL011", address_str="0x9000000", properties={})]

    with patch("subprocess.run", side_effect=FileNotFoundError):
        with pytest.raises(SystemExit) as e:
            validate_dtb("dummy.dtb", devices)
        assert e.value.code == 1


    assert "ERROR: 'dtc' (device-tree-compiler) not found" in caplog.text


def test_yaml2qemu_main_basic(tmp_path):
    yaml_content = """
machine:
  cpus:
    - name: cpu0
      type: cortex-a15
peripherals:
  - name: uart0
    type: UART.PL011
    address: 0x09000000
"""
    yaml_file = tmp_path / "test.yaml"
    yaml_file.write_text(yaml_content)
    dtb_file = tmp_path / "test.dtb"
    arch_file = tmp_path / "test.arch"

    test_args = ["yaml2qemu.py", str(yaml_file), "--out-dtb", str(dtb_file), "--out-arch", str(arch_file)]

    with (
        patch("sys.argv", test_args),
        patch("tools.yaml2qemu.compile_dtb", return_value=True),
        patch("tools.yaml2qemu.validate_dtb"),
    ):
        main()

    assert Path(arch_file).exists()
    with Path(arch_file).open() as f:
        assert f.read() == "arm"


def test_yaml2qemu_chardev_filtering(tmp_path):
    yaml_content = """
machine:
  cpus: []
peripherals:
  - name: serial0
    type: chardev
    address: none
    properties:
      node: "1"
"""
    yaml_file = tmp_path / "test_chardev.yaml"
    yaml_file.write_text(yaml_content)
    dtb_file = tmp_path / "test.dtb"
    cli_file = tmp_path / "test.cli"

    test_args = ["yaml2qemu.py", str(yaml_file), "--out-dtb", str(dtb_file), "--out-cli", str(cli_file)]

    with (
        patch("sys.argv", test_args),
        patch("tools.yaml2qemu.compile_dtb", return_value=True),
        patch("tools.yaml2qemu.validate_dtb"),
    ):
        main()

    assert Path(cli_file).exists()
    with Path(cli_file).open() as f:
        content = f.read()
        assert "-chardev" in content
        assert "virtmcu,id=chr_serial0,node=1" in content
        assert "-serial" in content
        assert "chardev:chr_serial0" in content
