import json
import os
import tempfile
import xml.etree.ElementTree as ET

from tools.svd2virtmcu.svd2header import generate_header, get_base_type
from tools.svd2virtmcu.svd2schema import generate_schema


def create_mock_svd(path: str, include_address_block: bool = True) -> None:
    root = ET.Element("device", schemaVersion="1.3")
    ET.SubElement(root, "name").text = "MockDevice"

    periphs = ET.SubElement(root, "peripherals")
    periph = ET.SubElement(periphs, "peripheral")
    ET.SubElement(periph, "name").text = "MOCK_IO"
    ET.SubElement(periph, "baseAddress").text = "0x10000000"

    if include_address_block:
        addr_block = ET.SubElement(periph, "addressBlock")
        ET.SubElement(addr_block, "offset").text = "0"
        ET.SubElement(addr_block, "size").text = "0x2000"

    regs = ET.SubElement(periph, "registers")

    reg1 = ET.SubElement(regs, "register")
    ET.SubElement(reg1, "name").text = "QPOS0"
    ET.SubElement(reg1, "description").text = "Shoulder float position"
    ET.SubElement(reg1, "addressOffset").text = "0x10"

    reg2 = ET.SubElement(regs, "register")
    ET.SubElement(reg2, "name").text = "TARGET0"
    ET.SubElement(reg2, "description").text = "Target position"
    ET.SubElement(reg2, "addressOffset").text = "0x100"

    # Edge case: Register with no description
    reg3 = ET.SubElement(regs, "register")
    ET.SubElement(reg3, "name").text = "UNKNOWN_REG"
    ET.SubElement(reg3, "addressOffset").text = "0x200"

    # Edge case: Intentionally non-target/non-qpos name for schema exclusion
    reg4 = ET.SubElement(regs, "register")
    ET.SubElement(reg4, "name").text = "STATUS"
    ET.SubElement(reg4, "addressOffset").text = "0x204"

    tree = ET.ElementTree(root)
    tree.write(path)


def test_svd2header_base_type() -> None:
    # Happy Path
    assert get_base_type("This is a float register") == "float"
    assert get_base_type("This measures in rad") == "float"
    assert get_base_type("Torque (nm)") == "float"

    # Corner Cases
    assert get_base_type("Status register") == "uint32_t"
    assert get_base_type(None) == "uint32_t"
    assert get_base_type("") == "uint32_t"
    assert get_base_type("Random garbage text") == "uint32_t"


def test_svd2header_generation_happy_path() -> None:
    with tempfile.TemporaryDirectory() as d:
        svd_path = os.path.join(d, "mock.svd")
        out_path = os.path.join(d, "out.h")
        template_path = os.path.join("tools", "svd2virtmcu", "templates", "c_header.j2")

        create_mock_svd(svd_path, include_address_block=True)
        generate_header(svd_path, template_path, out_path)

        assert os.path.exists(out_path)
        with open(out_path) as f:
            content = f.read()

            # Assert Enterprise SOTA checks are present
            assert "_Static_assert(sizeof(float) == 4" in content

            # Assert Register offsets and pointers are generated
            assert "#define REG_QPOS0_OFFSET 0x0010" in content
            assert "#define REG_QPOS0_PTR ((volatile float*)(MOCK_IO_BASE + REG_QPOS0_OFFSET))" in content

            # Verify size extraction worked
            assert "#define MOCK_IO_SIZE 0x2000" in content


def test_svd2header_generation_no_address_block() -> None:
    with tempfile.TemporaryDirectory() as d:
        svd_path = os.path.join(d, "mock.svd")
        out_path = os.path.join(d, "out.h")
        template_path = os.path.join("tools", "svd2virtmcu", "templates", "c_header.j2")

        # Create SVD with missing addressBlock to test fallback logic
        create_mock_svd(svd_path, include_address_block=False)
        generate_header(svd_path, template_path, out_path)

        with open(out_path) as f:
            content = f.read()
            # Default fallback size should be 0x1000
            assert "#define MOCK_IO_SIZE 0x1000" in content


def test_svd2schema_generation() -> None:
    with tempfile.TemporaryDirectory() as d:
        svd_path = os.path.join(d, "mock.svd")
        out_path = os.path.join(d, "schema.json")
        template_path = os.path.join("tools", "svd2virtmcu", "templates", "ui_schema.json.j2")

        create_mock_svd(svd_path)
        generate_schema(svd_path, template_path, out_path, world_id="test_world")

        assert os.path.exists(out_path)
        with open(out_path) as f:
            data = json.load(f)
            assert data["world_id"] == "test_world"
            node = data["nodes"]["cyber-arm-ctrl"]  # LINT_EXCEPTION: test fixture

            # We expect TARGET0 to map to Shoulder Target
            assert len(node["controls"]) == 1
            assert node["controls"][0]["id"] == "shoulder_target"
            assert node["controls"][0]["offset"] == 0x100

            # We expect QPOS0 to map to Shoulder Angle telemetry
            assert len(node["telemetry"]) == 1
            assert node["telemetry"][0]["id"] == "shoulder_angle"
