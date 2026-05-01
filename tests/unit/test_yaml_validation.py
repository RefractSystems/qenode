"""
SOTA Test Module: test_yaml_validation

Context:
This module implements tests for the test_yaml_validation subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_yaml_validation.
"""

import subprocess
import tempfile
import unittest
from pathlib import Path

import yaml


class TestYamlValidation(unittest.TestCase):
    def test_missing_peripheral_validation(self):
        """
        Tests that yaml2qemu fails if a peripheral in YAML is missing in the DTB.
        We simulate this by providing a YAML with a peripheral type that FdtEmitter skips.
        """
        with tempfile.TemporaryDirectory() as tmpdir:
            yaml_path = Path(tmpdir) / "test.yaml"
            dtb_path = Path(tmpdir) / "test.dtb"

            # Create a YAML with one valid peripheral and one 'Unknown' type which FdtEmitter skips
            test_yaml = {
                "machine": {"cpus": [{"name": "cpu0", "type": "cortex-a15"}]},
                "peripherals": [
                    {"name": "uart0", "type": "UART.PL011", "address": 0x10000000},
                    {
                        "name": "broken_dev",
                        "type": "UART.UnknownUART",  # FdtEmitter will skip this because of '.'
                        "address": 0x20000000,
                    },
                ],
            }

            with Path(yaml_path).open("w") as f:
                yaml.dump(test_yaml, f)

            # Run yaml2qemu
            # It should fail because 'broken_dev' will be missing in DTB
            cmd = ["python3", "-m", "tools.yaml2qemu", str(yaml_path), "--out-dtb", str(dtb_path)]

            result = subprocess.run(cmd, capture_output=True, text=True)
            output = result.stdout + result.stderr

            self.assertNotEqual(result.returncode, 0, "yaml2qemu should have failed")
            self.assertIn(
                "ERROR: The following peripherals from YAML are missing in the generated DTB: broken_dev", output
            )
            self.assertIn("FAILED: DTB validation failed.", output)

    def test_successful_validation(self):
        """
        Tests that yaml2qemu succeeds when all peripherals are correctly mapped.
        """
        with tempfile.TemporaryDirectory() as tmpdir:
            yaml_path = Path(tmpdir) / "test.yaml"
            dtb_path = Path(tmpdir) / "test.dtb"

            test_yaml = {
                "machine": {"cpus": [{"name": "cpu0", "type": "cortex-a15"}]},
                "peripherals": [{"name": "uart0", "type": "UART.PL011", "address": 0x10000000}],
            }

            with Path(yaml_path).open("w") as f:
                yaml.dump(test_yaml, f)

            cmd = ["python3", "-m", "tools.yaml2qemu", str(yaml_path), "--out-dtb", str(dtb_path)]

            result = subprocess.run(cmd, capture_output=True, text=True)
            output = result.stdout + result.stderr

            self.assertEqual(result.returncode, 0, f"yaml2qemu failed: {result.stderr}")
            self.assertIn("✓ Validation successful.", output)

    def test_topology_validation_success(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            yaml_path = Path(tmpdir) / "test.yaml"
            dtb_path = Path(tmpdir) / "test.dtb"

            test_yaml = {
                "nodes": [{"id": "node1"}],
                "topology": {
                    "max_messages_per_node_per_quantum": 500,
                    "global_seed": 42,
                    "links": [{"nodes": ["node1"]}],
                },
                "machine": {"cpus": [{"name": "cpu0", "type": "cortex-a15"}]},
            }

            with Path(yaml_path).open("w") as f:
                yaml.dump(test_yaml, f)

            cmd = ["python3", "-m", "tools.yaml2qemu", str(yaml_path), "--out-dtb", str(dtb_path)]
            result = subprocess.run(cmd, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, f"yaml2qemu failed: {result.stderr}")

    def test_topology_validation_invalid_max_msgs(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            yaml_path = Path(tmpdir) / "test.yaml"
            dtb_path = Path(tmpdir) / "test.dtb"

            test_yaml = {
                "nodes": [{"id": "node1"}],
                "topology": {
                    "max_messages_per_node_per_quantum": -1,
                    "global_seed": 42,
                },
                "machine": {"cpus": [{"name": "cpu0", "type": "cortex-a15"}]},
            }

            with Path(yaml_path).open("w") as f:
                yaml.dump(test_yaml, f)

            cmd = ["python3", "-m", "tools.yaml2qemu", str(yaml_path), "--out-dtb", str(dtb_path)]
            result = subprocess.run(cmd, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("max_messages_per_node_per_quantum must be a positive integer", result.stderr)

    def test_topology_validation_unknown_node(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            yaml_path = Path(tmpdir) / "test.yaml"
            dtb_path = Path(tmpdir) / "test.dtb"

            test_yaml = {
                "nodes": [{"id": "node1"}],
                "topology": {"links": [{"nodes": ["node1", "unknown_node"]}]},
                "machine": {"cpus": [{"name": "cpu0", "type": "cortex-a15"}]},
            }

            with Path(yaml_path).open("w") as f:
                yaml.dump(test_yaml, f)

            cmd = ["python3", "-m", "tools.yaml2qemu", str(yaml_path), "--out-dtb", str(dtb_path)]
            result = subprocess.run(cmd, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Topology validation failed: node ID unknown_node in links not found in nodes", result.stderr)


if __name__ == "__main__":
    unittest.main()
