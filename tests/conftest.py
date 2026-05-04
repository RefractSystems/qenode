from __future__ import annotations

import ast
import pathlib
from collections.abc import AsyncGenerator
from typing import TYPE_CHECKING

import pytest
import pytest_asyncio

from tools.testing.virtmcu_test_suite.conftest_core import (
    VirtualTimeAuthority,
    deterministic_coordinator,
    deterministic_coordinator_bin,
    get_free_port,
    guest_app_factory,
    inspection_bridge,
    pytest_collection_modifyitems,
    pytest_runtest_makereport,
    qemu_launcher,
    qmp_bridge,
    script_runner,
    simulation,
    time_authority,
    zenoh_router,
    zenoh_session,
)
from tools.testing.virtmcu_test_suite.transport import ZenohTransportImpl

if TYPE_CHECKING:
    import zenoh

    from tools.testing.virtmcu_test_suite.conftest_core import VirtualTimeAuthority

# Re-exporting fixtures so pytest finds them
__all__ = [
    "VirtualTimeAuthority",
    "pytest_collection_modifyitems",
    "pytest_runtest_makereport",
    "qemu_launcher",
    "qmp_bridge",
    "script_runner",
    "inspection_bridge",
    "simulation",
    "time_authority",
    "deterministic_coordinator",
    "deterministic_coordinator_bin",
    "get_free_port",
    "guest_app_factory",
    "zenoh_router",
    "zenoh_session",
]


@pytest_asyncio.fixture
async def _sim_transport_zenoh(zenoh_router: str, zenoh_session: zenoh.Session) -> AsyncGenerator[ZenohTransportImpl]:
    transport = ZenohTransportImpl(zenoh_router, zenoh_session)
    await transport.start()
    yield transport
    await transport.stop()


@pytest_asyncio.fixture(params=["zenoh"])
async def sim_transport(
    request: pytest.FixtureRequest,
    _sim_transport_zenoh: ZenohTransportImpl,
) -> AsyncGenerator[ZenohTransportImpl]:
    yield _sim_transport_zenoh


# ---------------------------------------------------------------------------
# GLOBAL SUBPROCESS GUARD
# Standard VirtMCU tests MUST NOT spawn background processes manually.
# Use standard fixtures instead.
# ---------------------------------------------------------------------------

_BANNED_SUBPROCESS_ATTRS = frozenset({"run", "check_output", "Popen", "call", "check_call", "getstatusoutput", "getoutput"})
_BANNED_ASYNCIO_ATTRS = frozenset({"create_subprocess_exec", "create_subprocess_shell"})
_BANNED_OS_ATTRS = frozenset({"system", "popen", "spawnl", "spawnle", "spawnlp", "spawnlpe", "spawnv", "spawnve", "spawnvp", "spawnvpe"})
_BANNED_GLOBAL_NAMES = frozenset({"Popen", "create_subprocess_exec", "create_subprocess_shell", "system", "popen"})


def _scan_subprocess_in_test_bodies(root: pathlib.Path) -> list[str]:
    violations: list[str] = []
    for path in sorted(root.rglob("test_*.py")):
        # Skip hidden directories, virtual environments, fixtures, third_party, and build artifacts
        if (
            any(part.startswith(".") for part in path.parts)
            or "fixtures" in path.parts
            or "venv" in path.parts
            or "third_party" in path.parts
            or "target" in path.parts
            or "build" in path.parts
            or "_deps" in path.parts
        ):
            continue
        try:
            with path.open("r", encoding="utf-8") as f:
                tree = ast.parse(f.read(), filename=str(path))
                for node in ast.walk(tree):
                    if isinstance(node, ast.Call):
                        if isinstance(node.func, ast.Attribute) and isinstance(node.func.value, ast.Name):
                            module_name = node.func.value.id
                            attr_name = node.func.attr
                            if module_name == "subprocess" and attr_name in _BANNED_SUBPROCESS_ATTRS:
                                violations.append(
                                    f"{path.name}:{node.lineno} — Manually spawning subprocess.{attr_name}. Move to a fixture or factory.py"
                                )
                            elif module_name == "asyncio" and attr_name in _BANNED_ASYNCIO_ATTRS:
                                violations.append(
                                    f"{path.name}:{node.lineno} — Manually spawning asyncio.{attr_name}. Move to a fixture or factory.py"
                                )
                            elif module_name == "os" and attr_name in _BANNED_OS_ATTRS:
                                violations.append(
                                    f"{path.name}:{node.lineno} — Manually spawning os.{attr_name}. Use subprocess via fixtures instead"
                                )
                        elif isinstance(node.func, ast.Name):
                            func_name = node.func.id
                            if func_name in _BANNED_GLOBAL_NAMES:
                                violations.append(
                                    f"{path.name}:{node.lineno} — Manually spawning {func_name}. Move to a fixture with teardown"
                                )
        except (OSError, SyntaxError, ValueError) as e:
            violations.append(f"Error parsing {path}: {e}")
    return violations


def pytest_sessionstart(session: pytest.Session) -> None:
    """Session start hook."""
    violations = _scan_subprocess_in_test_bodies(session.config.rootpath)
    if violations:
        print("\n" + "=" * 80)  # noqa: T201
        print("ERROR: Manual subprocess spawning detected in test bodies.")  # noqa: T201
        print("Standard VirtMCU tests MUST NOT spawn processes manually.")  # noqa: T201
        print("Use standard fixtures (qemu_launcher, deterministic_coordinator, etc.) instead.")  # noqa: T201
        print("-" * 80)  # noqa: T201
        for v in violations:
            print(f"  * {v}")  # noqa: T201
        print("=" * 80 + "\n")  # noqa: T201
        pytest.exit("Aborting due to simulation hygiene violations", returncode=1)
