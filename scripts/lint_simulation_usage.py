#!/usr/bin/env python3
"""
AST-based lint for enforcing VirtMCU simulation framework usage.
Banned: manual ensure_session_routing, manual qemu_launcher,
and manual -S in extra_args.
"""

from __future__ import annotations

import ast
import sys
from pathlib import Path


def lint_file(path: Path) -> list[str]:
    with path.open("r") as f:
        tree = ast.parse(f.read(), filename=str(path))

    violations = []
    for node in ast.walk(tree):
        # 1. Banned: ensure_session_routing in test body
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and node.func.id == "ensure_session_routing":
            # conftest_core.py is exempt (it defines the helper)
            if path.name not in ("conftest_core.py", "simulation.py"):
                violations.append(
                    f"{path}:{node.lineno}: Banned manual ensure_session_routing. "
                    "Routing synchronization is handled automatically by the simulation fixture."
                )

        # 2. Banned: qemu_launcher in test body (unless using simulation fixture)
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and node.func.id == "qemu_launcher":
            if path.name not in ("conftest_core.py", "simulation.py"):
                violations.append(
                    f"{path}:{node.lineno}: Banned manual qemu_launcher. "
                    "Use the `simulation` fixture for multi-node tests or `qmp_bridge` for single-node tests."
                )

        # 3. Banned: -S in extra_args (handled by framework)
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute) and node.func.attr == "add_node":
            for kw in node.keywords:
                if kw.arg == "extra_args" and isinstance(kw.value, ast.List):
                    for elt in kw.value.elts:
                        if isinstance(elt, ast.Constant) and elt.value == "-S":
                            violations.append(
                                f"{path}:{node.lineno}: Banned manual '-S' in extra_args. "
                                "QEMU is launched frozen by default; the framework handles the boot sequence."
                            )

        # 4. Banned: raw subprocess in test body (for orchestration)
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute) and node.func.attr == "Popen":
            if isinstance(node.func.value, ast.Name) and node.func.value.id == "subprocess":
                # Only check tests/
                if "tests" in path.parts:
                    violations.append(
                        f"{path}:{node.lineno}: Banned manual subprocess.Popen in tests. "
                        "Use `ManagedSubprocess` from conftest_core.py for deterministic cleanup."
                    )

        # 5. Banned: raw string lookup for core YAML keys
        if isinstance(node, ast.Subscript):
            if isinstance(node.slice, ast.Constant) and isinstance(node.slice.value, str):
                val = node.slice.value
                if val in ("peripherals", "topology", "machine", "memory", "nodes"):
                    if path.name not in ("world_schema.py", "yaml2qemu.py"):
                        with path.open("r") as f:
                            lines = f.readlines()
                        if node.lineno <= len(lines) and "LINT_EXCEPTION" not in lines[node.lineno - 1]:
                            violations.append(
                                f"{path}:{node.lineno}: Banned raw string lookup for YAML key '{val}'. "
                                "Use the `WorldYaml` Pydantic model from "
                                "`tools.testing.virtmcu_test_suite.generated` instead."
                            )

        if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute) and node.func.attr == "get":
            if node.args and isinstance(node.args[0], ast.Constant) and isinstance(node.args[0].value, str):
                val = node.args[0].value
                if val in ("peripherals", "topology", "machine", "memory", "nodes"):
                    if path.name not in ("world_schema.py", "yaml2qemu.py"):
                        with path.open("r") as f:
                            lines = f.readlines()
                        if node.lineno <= len(lines) and "LINT_EXCEPTION" not in lines[node.lineno - 1]:
                            violations.append(
                                f"{path}:{node.lineno}: Banned .get('{val}') for YAML key. "
                                "Use the `WorldYaml` Pydantic model from "
                                "`tools.testing.virtmcu_test_suite.generated` instead."
                            )

        # Ban raw subprocess.Popen
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute):
            if node.func.attr == "Popen" and isinstance(node.func.value, ast.Name) and node.func.value.id == "subprocess":
                with path.open("r") as f:
                    lines = f.readlines()
                if node.lineno <= len(lines) and "LINT_EXCEPTION" not in lines[node.lineno - 1]:
                    violations.append(
                        f"{path}:{node.lineno}: Banned raw subprocess.Popen. "
                        "Use ManagedSubprocess for deterministic lifecycle management and unified logging."
                    )

        # Ban vta.step inside loops
        if isinstance(node, (ast.For, ast.While)):
            for subnode in ast.walk(node):
                if isinstance(subnode, ast.Call) and isinstance(subnode.func, ast.Attribute) and subnode.func.attr == "step":
                    if isinstance(subnode.func.value, ast.Attribute) and subnode.func.value.attr in ("vta", "clock"):
                        with path.open("r") as f:
                            lines = f.readlines()
                        if subnode.lineno <= len(lines) and "LINT_EXCEPTION: vta_step_loop" not in lines[subnode.lineno - 1]:
                            violations.append(
                                f"{path}:{subnode.lineno}: Banned vta.step() inside a loop. "
                                "This is polling. Use sim.run_until() or node.wait_for_uart() instead. "
                                "If this is a deterministic iteration over quanta, add '# LINT_EXCEPTION: vta_step_loop'."
                            )
                elif isinstance(subnode, ast.Call) and isinstance(subnode.func, ast.Attribute) and subnode.func.attr == "sleep":
                    if isinstance(subnode.func.value, ast.Name) and subnode.func.value.id in ("asyncio", "time"):
                        with path.open("r") as f:
                            lines = f.readlines()
                        if subnode.lineno <= len(lines) and "SLEEP_EXCEPTION" not in lines[subnode.lineno - 1]:
                            violations.append(
                                f"{path}:{subnode.lineno}: Banned sleep() without SLEEP_EXCEPTION. "
                                "Sleeping is banned. Use deterministic barriers."
                            )

    return violations


def main() -> None:
    root = Path("/workspace")
    tests_dir = root / "tests"
    tools_testing_dir = root / "tools/testing"

    all_violations = []

    for path in sorted(list(tests_dir.rglob("*.py")) + list(tools_testing_dir.rglob("*.py"))):
        if "fixtures" in path.parts or "__pycache__" in path.parts:
            continue
        all_violations.extend(lint_file(path))

    if all_violations:
        for v in all_violations:
            sys.stdout.write(f"{v}\n")
        sys.exit(1)
    else:
        sys.stdout.write("Simulation usage lint passed.\n")
        sys.exit(0)


if __name__ == "__main__":
    main()
