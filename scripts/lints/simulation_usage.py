#!/usr/bin/env python3
"""
AST-based lint for enforcing VirtMCU simulation framework usage.
Ensures that tests and orchestration code use the correct simulation fixtures
and avoid manual QEMU execution, raw subprocesses, and polling.
Enforces Enterprise-Grade SOTA software quality by strictly forbidding comment escapes.

Designed for reuse: Can be run as a CLI tool or imported as a module by parent repositories.
"""

from __future__ import annotations

import argparse
import ast
import logging
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))  # noqa: TID251
from lint_utils import DEFAULT_EXCLUDES, ENTERPRISE_MANDATE, iter_target_files, setup_lint_logging

logger = logging.getLogger(__name__)


def lint_file(path: Path) -> list[str]:
    try:
        with path.open("r") as f:
            tree = ast.parse(f.read(), filename=str(path))
    except (SyntaxError, ValueError) as e:
        return [f"{path}: Failed to parse AST: {e}"]

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

        # Ban manual struct packing and int.from_bytes (Mandate 10)
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute):
            if node.func.attr == "from_bytes" and isinstance(node.func.value, ast.Name) and node.func.value.id == "int":
                with path.open("r") as f:
                    lines = f.readlines()
                if node.lineno <= len(lines) and "LINT_EXCEPTION: int_from_bytes" not in lines[node.lineno - 1]:
                    violations.append(
                        f"{path}:{node.lineno}: Banned manual protocol deserialization via int.from_bytes. "
                        f"Use vproto.py FlatBuffer wrappers. {ENTERPRISE_MANDATE} '# LINT_EXCEPTION: int_from_bytes'."
                    )
            elif (
                node.func.attr in ("pack", "unpack", "unpack_from")
                and isinstance(node.func.value, ast.Name)
                and node.func.value.id == "struct"
            ):
                with path.open("r") as f:
                    lines = f.readlines()
                if node.lineno <= len(lines) and "LINT_EXCEPTION: struct_pack" not in lines[node.lineno - 1]:
                    violations.append(
                        f"{path}:{node.lineno}: Banned manual struct packing/unpacking. "
                        "Use vproto.py FlatBuffer wrappers."
                    )

        # Ban raw subprocess.Popen
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute):
            if (
                node.func.attr == "Popen"
                and isinstance(node.func.value, ast.Name)
                and node.func.value.id == "subprocess"
            ):
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
                if (
                    isinstance(subnode, ast.Call)
                    and isinstance(subnode.func, ast.Attribute)
                    and subnode.func.attr == "step"
                ):
                    if isinstance(subnode.func.value, ast.Attribute) and subnode.func.value.attr in ("vta", "clock"):
                        with path.open("r") as f:
                            lines = f.readlines()
                        if (
                            subnode.lineno <= len(lines)
                            and "LINT_EXCEPTION: vta_step_loop" not in lines[subnode.lineno - 1]
                        ):
                            violations.append(
                                f"{path}:{subnode.lineno}: Banned vta.step() inside a loop. "
                                "This is polling. Use sim.run_until() or node.wait_for_uart() instead. "
                                f"{ENTERPRISE_MANDATE} '# LINT_EXCEPTION: vta_step_loop'."
                            )
                elif (
                    isinstance(subnode, ast.Call)
                    and isinstance(subnode.func, ast.Attribute)
                    and subnode.func.attr == "sleep"
                ):
                    if isinstance(subnode.func.value, ast.Name) and subnode.func.value.id in ("asyncio", "time"):
                        with path.open("r") as f:
                            lines = f.readlines()
                        if subnode.lineno <= len(lines) and "SLEEP_EXCEPTION" not in lines[subnode.lineno - 1]:
                            violations.append(
                                f"{path}:{subnode.lineno}: Banned sleep() without SLEEP_EXCEPTION. "
                                f"Sleeping is banned. Use deterministic barriers. {ENTERPRISE_MANDATE} '# SLEEP_EXCEPTION: <reason>'."
                            )

    return violations


def run_lint(targets: list[Path], excludes: list[str]) -> bool:
    """Executes the linting process. Returns True if passed, False if violations found."""
    all_violations = []

    for path in iter_target_files(targets, excludes, "*.py"):
        if "fixtures" in path.parts or "__pycache__" in path.parts:
            continue

        all_violations.extend(lint_file(path))

    if all_violations:
        for v in all_violations:
            logger.error(v)
        return False

    logger.info("✓ Simulation usage lint passed.")
    return True


def main() -> None:
    parser = argparse.ArgumentParser(description="Lint Simulation Framework Usage via AST.")
    parser.add_argument(
        "targets",
        type=Path,
        nargs="*",
        default=[Path("tests"), Path("tools/testing")],
        help="Target directories or files",
    )
    parser.add_argument("--exclude", nargs="*", default=DEFAULT_EXCLUDES, help="Directories to exclude")
    args = parser.parse_args()

    setup_lint_logging()
    success = run_lint(args.targets, args.exclude)
    sys.exit(0 if success else 1)


if __name__ == "__main__":
    main()
