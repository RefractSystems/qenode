from pathlib import Path

import yaml

from tools.testing.virtmcu_test_suite.generated import WorldSchema


def test_parse_pendulum_yaml() -> None:
    yaml_path = Path("worlds/pendulum.yml")
    assert yaml_path.exists(), "Pendulum YAML not found"
    with open(yaml_path) as f:
        _data = yaml.safe_load(f)
    # Pendulum is a docker-compose file technically in the root, wait, worlds/pendulum.yml is a docker-compose file!
    # We should test a true world yaml like tests/fixtures/guest_apps/yaml_boot/test_board.yaml


def test_parse_test_board_yaml() -> None:
    yaml_path = Path("tests/fixtures/guest_apps/yaml_boot/test_board.yaml")
    assert yaml_path.exists(), "Test board YAML not found"

    with open(yaml_path) as f:
        data = yaml.safe_load(f)

    world = WorldSchema.model_validate(data)

    assert world.machine is not None
    assert world.machine.name == "test_board"
    assert world.machine.type == "arm-generic-fdt"
    assert world.machine.cpus is not None
    assert len(world.machine.cpus) == 1
    assert world.machine.cpus[0].name == "cpu"
    assert world.machine.cpus[0].type == "cortex-a15"
    assert world.machine.cpus[0].memory == "sysmem"

    assert world.peripherals is not None
    assert len(world.peripherals) == 3

    uart = next(p for p in world.peripherals if p.name.root == "uart0")
    assert uart.renode_type == "UART.PL011"

    # Address validation (hex string to int or raw string, currently generated model allows int | str)
    assert uart.address is not None
    assert uart.address.root == "0x09000000"
    assert uart.interrupts == ["gic@1"]


def test_parse_lin_topology() -> None:
    yaml_path = Path("tests/fixtures/topologies/lin_2node.yml")
    assert yaml_path.exists(), "LIN topology YAML not found"

    with open(yaml_path) as f:
        data = yaml.safe_load(f)

    world = WorldSchema.model_validate(data)

    assert world.topology is not None
    assert world.topology.nodes is not None
    assert len(world.topology.nodes) == 2

    assert world.topology.nodes[0].name.root == "0"
    assert world.topology.nodes[1].name.root == "1"

    assert world.topology.links is not None
    assert len(world.topology.links) == 1

    link = world.topology.links[0]
    assert link.type.root == "lin"
    assert [n.root for n in link.nodes] == ["0", "1"]
