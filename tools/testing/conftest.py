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
    inspection_bridge,
    pytest_collection_modifyitems,
    pytest_runtest_makereport,
    qemu_launcher,
    qmp_bridge,
    simulation,
    time_authority,
    zenoh_router,
    zenoh_session,
)
from tools.testing.virtmcu_test_suite.transport import UnixTransportImpl, ZenohTransportImpl

if TYPE_CHECKING:
    import zenoh

    from tools.testing.virtmcu_test_suite.conftest_core import VirtualTimeAuthority

# Re-exporting fixtures so pytest finds them
__all__ = [
    "VirtualTimeAuthority",
    "inspection_bridge",
    "pytest_collection_modifyitems",
    "pytest_runtest_makereport",
    "qemu_launcher",
    "qmp_bridge",
    "simulation",
    "time_authority",
    "deterministic_coordinator",
    "zenoh_router",
    "zenoh_session",
]


@pytest_asyncio.fixture
async def _sim_transport_zenoh(zenoh_router: str, zenoh_session: zenoh.Session) -> AsyncGenerator[ZenohTransportImpl]:
    transport = ZenohTransportImpl(zenoh_router, zenoh_session)
    await transport.start()
    yield transport
    await transport.stop()


@pytest_asyncio.fixture
async def _sim_transport_unix() -> AsyncGenerator[UnixTransportImpl]:
    transport = UnixTransportImpl()
    await transport.start()
    yield transport
    await transport.stop()


@pytest_asyncio.fixture(params=["zenoh", "unix"])
async def sim_transport(
    request: pytest.FixtureRequest,
    _sim_transport_zenoh: ZenohTransportImpl,
    _sim_transport_unix: UnixTransportImpl,
) -> AsyncGenerator[ZenohTransportImpl | UnixTransportImpl]:
    if request.param == "zenoh":
        yield _sim_transport_zenoh
    else:
        yield _sim_transport_unix


# ---------------------------------------------------------------------------
# GLOBAL SUBPROCESS GUARD
# Standard VirtMCU tests MUST NOT spawn background processes manually.
# Use standard fixtures instead.
# ---------------------------------------------------------------------------

_BANNED_SPAWN_NAMES = frozenset({"Popen", "create_subprocess_exec", "create_subprocess_shell"})


def _scan_subprocess_in_test_bodies(root: pathlib.Path) -> list[str]:
    violations: list[str] = []
    for path in sorted(root.rglob("test_*.py")):
        if "fixtures" in path.parts:
            continue
        try:
            with path.open("r", encoding="utf-8") as f:
                tree = ast.parse(f.read(), filename=str(path))
                for node in ast.walk(tree):
                    if isinstance(node, ast.Call):
                        name = ""
                        if isinstance(node.func, ast.Name):
                            name = node.func.id
                        elif isinstance(node.func, ast.Attribute):
                            name = node.func.attr

                        if name in _BANNED_SPAWN_NAMES:
                            violations.append(
                                f"{path.name}:{node.lineno} — Manually spawning {name}. Move to a fixture with teardown"
                            )
        except Exception as e:  # noqa: BLE001
            violations.append(f"Error parsing {path}: {e}")
    return violations
