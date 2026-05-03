import pytest

from tools.testing.virtmcu_test_suite.world_schema import WorldYaml


def test_schema_modern() -> None:
    content = """
topology:
  nodes:
    - name: "1"
    - name: 2
  links:
    - type: uart
      nodes: [1, 2]
"""
    world = WorldYaml.from_text(content)
    assert world.topology is not None
    assert world.topology.nodes is not None
    assert len(world.topology.nodes) == 2
    assert world.get_node_ids() == {"1", "2"}


def test_schema_legacy_fallback() -> None:
    content = """
peripherals:
  - name: "1"
    type: clock
  - name: 2
    type: uart
"""
    world = WorldYaml.from_text(content)
    assert world.topology is None
    assert world.get_node_ids() == {"1", "2"}


def test_schema_machine_passthrough() -> None:
    content = """
machine:
  type: arm
peripherals:
  - name: uart0
    type: pl011
"""
    world = WorldYaml.from_text(content)
    assert world.topology is None
    assert world.get_node_ids() == set()


def test_schema_split_brain_rejection_nodes() -> None:
    content = """
nodes:
  - name: "1"
topology:
  nodes:
    - name: "2"
"""
    with pytest.raises(ValueError, match="Split-brain YAML detected"):
        WorldYaml.from_text(content)


def test_schema_split_brain_rejection_numeric_peripherals() -> None:
    content = """
peripherals:
  - name: "1"
topology:
  nodes:
    - name: "2"
"""
    with pytest.raises(ValueError, match="Split-brain YAML detected"):
        WorldYaml.from_text(content)


def test_schema_machine_peripherals_with_topology() -> None:
    content = """
peripherals:
  - name: uart0
    type: pl011
topology:
  nodes:
    - name: "1"
"""
    # This should be fine as 'uart0' is not numeric
    world = WorldYaml.from_text(content)
    assert world.get_node_ids() == {"1"}
