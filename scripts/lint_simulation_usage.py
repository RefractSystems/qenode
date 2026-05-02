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
    violations = []
    try:
        with path.open("r", encoding="utf-8") as f:
            content = f.read()
            tree = ast.parse(content, filename=str(path))
    except (OSError, SyntaxError, ValueError) as e:
        return [f"{path}:0: Error parsing file: {e}"]

    # Files exempt from the hardcoded-topic rule:
    #   - topics.py: the registry itself.
    #   - test_topic_registry.py: the schema-contract test that pins the
    #     literal strings against `SimTopic.*` so any divergence with the
    #     Rust coordinator's wildcards is caught.
    #   - vproto / test_vproto: wire-format helpers that legitimately reference
    #     protocol names (not Zenoh topics).
    topic_lint_exempt_files = {
        "topics.py",
        "test_topic_registry.py",
    }
    is_topic_exempt = path.name in topic_lint_exempt_files or "vproto.py" in path.name or "test_vproto" in path.name

    for node in ast.walk(tree):
        if isinstance(node, ast.Constant):
            if isinstance(node.value, str):
                val = node.value
                if val.startswith("sim/") or val.startswith("virtmcu/uart/"):
                    if not is_topic_exempt:
                        with path.open("r") as f:
                            lines = f.readlines()
                        if node.lineno > len(lines):
                            continue
                        line_text = lines[node.lineno - 1]
                        # Old `hardcoded_topic` reason was abused by a
                        # one-shot script (`fix_lint.py`) to silence the
                        # rule wholesale. It is no longer accepted.
                        if "LINT_EXCEPTION: hardcoded_topic_wildcard" in line_text:
                            # Wildcards (e.g. `sim/coord/*/done`) are the
                            # only legitimate inline literal: a fan-in
                            # subscriber MUST take the wildcard expression.
                            # Even so, prefer the `*_WILDCARD` constants
                            # exported by SimTopic.
                            if "*" not in val:
                                violations.append(
                                    f"{path}:{node.lineno}: 'hardcoded_topic_wildcard' "
                                    f"exception used on non-wildcard topic '{val}'. "
                                    "Use a `SimTopic.*` helper instead."
                                )
                            continue
                        if "LINT_EXCEPTION: hardcoded_topic" in line_text:
                            violations.append(
                                f"{path}:{node.lineno}: deprecated "
                                "'LINT_EXCEPTION: hardcoded_topic' is no longer "
                                "accepted (silenced by a removed one-shot script). "
                                "Replace the literal with a `SimTopic.*` helper, or "
                                "if it is genuinely a wildcard subscriber pattern, "
                                "use 'LINT_EXCEPTION: hardcoded_topic_wildcard'."
                            )
                            continue
                        violations.append(
                            f"{path}:{node.lineno}: Banned magic Zenoh topic string '{val}'. "
                            "Use the `SimTopic` registry from "
                            "`tools.testing.virtmcu_test_suite.topics` instead. "
                            "Wildcard subscriber patterns may carry "
                            "'# LINT_EXCEPTION: hardcoded_topic_wildcard'."
                        )

        # Rule 1, 2, 3: Banned function/class calls
        if isinstance(node, ast.Call):
            name = ""
            if isinstance(node.func, ast.Name):
                name = node.func.id
            elif isinstance(node.func, ast.Attribute):
                name = node.func.attr

            if name in ("get_rust_binary_path", "resolve_rust_binary"):
                # Ensure the first argument is a VirtmcuBinary attribute access, not a string
                if node.args and isinstance(node.args[0], ast.Constant):
                    arg_val = node.args[0].value
                    if isinstance(arg_val, str):
                        # Check for LINT_EXCEPTION: hardcoded_binary
                        with path.open("r") as f:
                            lines = f.readlines()
                            if (
                                node.lineno <= len(lines)
                                and "LINT_EXCEPTION: hardcoded_binary" not in lines[node.lineno - 1]
                            ):
                                violations.append(
                                    f"{path}:{node.lineno}: Banned hardcoded string '{arg_val}' in {name}(). "
                                    "Use the `VirtmcuBinary` enum from `tools.testing.virtmcu_test_suite.constants` instead. "
                                    "If this is for unit testing the resolver itself, use '# LINT_EXCEPTION: hardcoded_binary'."
                                )

            if name == "ensure_session_routing":
                # Hard-ban in tests. The framework handles routing for firmware
                # tests (`simulation` fixture) and for direct-coordinator tests
                # (`coordinator_subprocess` context manager). There is no
                # remaining legitimate caller in tests.
                if path.name not in ("conftest_core.py", "simulation.py"):
                    violations.append(
                        f"{path}:{node.lineno}: Banned call to ensure_session_routing(). "
                        "Use the `simulation` fixture (firmware tests) or "
                        "`coordinator_subprocess` context manager (direct-coordinator "
                        "tests). Both run the routing barrier internally."
                    )

            if name == "qemu_launcher":
                # Exception: conftest_core.py contains the only approved callers
                # (qmp_bridge, simulation fixture, and inspection_bridge).
                is_exception = path.name == "conftest_core.py"
                if not is_exception:
                    violations.append(
                        f"{path}:{node.lineno}: Banned call to qemu_launcher(). "
                        "Use simulation or inspection_bridge instead."
                    )

            if name in ("Simulation", "VirtmcuSimulation", "SimulationOrchestrator"):
                # Exception: conftest_core.py (fixture) or simulation.py (implementation).
                # `SimulationOrchestrator` was deleted but is kept here as a
                # tripwire — if a future change re-introduces it, the lint fires.
                is_exception = path.name in ("conftest_core.py", "simulation.py")
                if not is_exception:
                    violations.append(
                        f"{path}:{node.lineno}: Banned direct {name}() instantiation. "
                        "Use the simulation fixture instead."
                    )

        # Rule 4: Manual -S in extra_args
        if isinstance(node, ast.keyword) and node.arg == "extra_args":
            if isinstance(node.value, ast.List):
                for elt in node.value.elts:
                    if isinstance(elt, ast.Constant) and elt.value == "-S":
                        # Exception: internal framework code or QemuLibrary tests
                        is_exception = path.name in (
                            "conftest_core.py",
                            "simulation.py",
                            "test_device_realization.py",
                            "test_qemu_library_pytest.py",
                        )
                        if not is_exception:
                            violations.append(
                                f"{path}:{node.lineno}: Banned manual '-S' in extra_args. "
                                "The framework (Simulation or inspection_bridge) handles this."
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
            print(v)
        sys.exit(1)
    else:
        print("Simulation usage lint passed.")
        sys.exit(0)


if __name__ == "__main__":
    main()
