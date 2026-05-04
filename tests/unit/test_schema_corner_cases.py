import pytest
from pydantic import ValidationError

from tools.testing.virtmcu_test_suite.generated import WorldSchema


def test_reject_invalid_protocol() -> None:
    data = {"topology": {"links": [{"type": "invalid_protocol", "nodes": ["0", "1"]}]}}
    with pytest.raises(ValidationError) as exc_info:
        WorldSchema.model_validate(data)

    assert "Input should be 'Ethernet', 'Uart', 'CanFd', 'Spi', 'FlexRay', 'Lin', 'Rf802154' or 'RfHci'" in str(
        exc_info.value
    ) or "pattern" in str(exc_info.value)


def test_address_hex_string_validation() -> None:
    # Valid hex string
    data_valid = {"peripherals": [{"name": "valid_mem", "address": "0x1A2B3C", "size": 4096}]}
    world = WorldSchema.model_validate(data_valid)
    assert world.peripherals is not None
    assert world.peripherals[0].address is not None
    assert world.peripherals[0].address.root == "0x1A2B3C"

    # Invalid hex string
    data_invalid = {
        "peripherals": [
            {
                "name": "invalid_mem",
                "address": "1A2B3C",  # Missing 0x
            }
        ]
    }
    with pytest.raises(ValidationError):
        WorldSchema.model_validate(data_invalid)


def test_missing_required_fields() -> None:
    # Link missing 'nodes'
    data = {"topology": {"links": [{"type": "uart"}]}}
    with pytest.raises(ValidationError) as exc_info:
        WorldSchema.model_validate(data)
    assert "Field required" in str(exc_info.value)


def test_legacy_nodes_block() -> None:
    # Nodes defined at root level (legacy) should parse successfully
    data = {"nodes": [{"name": "0"}, {"name": "1"}]}
    world = WorldSchema.model_validate(data)
    assert world.nodes is not None
    assert len(world.nodes) == 2
    assert world.nodes[0].name.root == "0"
