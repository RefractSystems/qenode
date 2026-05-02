"""
SOTA single-entry-point simulation harness.

Subsumes legacy single-node and multi-node orchestrators
under one class with a strict, framework-owned lifecycle.

Lifecycle (see /workspace/docs/guide/03-testing-strategy.md §6):
  1. Spawn all QEMU nodes frozen (`-S` injected by qemu_launcher).
  2. Liveliness barrier (`vta.init()` waits for `sim/clock/liveliness/{nid}`)
     and 0-ns sync — performed while QEMU is still frozen.
  3. Router barrier (`ensure_session_routing(session)`).
  4. `cont` (start_emulation) issued to all nodes.
  5. Strict reverse-order teardown.

Use via the `simulation` pytest fixture defined in `conftest_core.py`.
Direct instantiation in tests is banned (see CLAUDE.md §SOTA).
"""

from __future__ import annotations

import asyncio
import logging
import os
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import TYPE_CHECKING, Any

from tools.testing.utils import get_time_multiplier
from tools.testing.virtmcu_test_suite.conftest_core import (
    VirtualTimeAuthority,
    ensure_session_routing,
    wait_for_zenoh_discovery,
)
from tools.testing.virtmcu_test_suite.topics import SimTopic

if TYPE_CHECKING:
    from collections.abc import Callable, Coroutine
    from pathlib import Path

    import zenoh

    from tools.testing.qmp_bridge import QmpBridge
    from tools.testing.virtmcu_test_suite.transport import SimulationTransport

logger = logging.getLogger(__name__)

# Known virtmcu plugins and their substrings for detection
# Note: 'clock' is handled natively by vta.init() and thus excluded from some loops.
_KNOWN_VIRTMCU_PLUGINS = [
    "chardev",
    "netdev",
    "spi",
    "canfd",
    "ieee802154",
    "flexray",
    "actuator",
    "ui",
    "telemetry",
    "s32k144-lpuart",
]


@dataclass
class _NodeSpec:
    node_id: int
    dtb: str | Path
    kernel: str | Path | None
    extra_args: list[str] = field(default_factory=list)
    orchestrated: bool = True

    @property
    def plugins(self) -> set[str]:
        plugins = set()
        import re

        import fdt

        # Mapping from QOM type names / compatible strings to liveliness plugin names
        type_to_plugin = {
            "can-host-virtmcu": "canfd",
            "virtmcu": "chardev",
            "virtmcu-chardev": "chardev",
            "netdev": "netdev",
            "spi": "spi",
            "ieee802154": "ieee802154",
            "flexray": "flexray",
            "actuator": "actuator",
            "ui": "ui",
            "telemetry": "telemetry",
            "s32k144-lpuart": "s32k144-lpuart",
        }

        for i, arg in enumerate(self.extra_args):
            # Detect via virtmcu- prefix (ensure it's a word boundary to avoid matching paths)
            m = re.search(r"\bvirtmcu-([a-zA-Z0-9_]+)", arg)
            if m:
                plugins.add(m.group(1))

            # Detect via explicit names in -device, -chardev, or -netdev
            if arg in ["-device", "-chardev", "-netdev"] and i + 1 < len(self.extra_args):
                val = self.extra_args[i + 1]
                type_name = val.split(",")[0]
                if type_name in type_to_plugin:
                    plugins.add(type_to_plugin[type_name])

            # Detect via -global
            elif arg == "-global" and i + 1 < len(self.extra_args):
                val = self.extra_args[i + 1]
                for t, p in type_to_plugin.items():
                    if val.startswith(f"{t}."):
                        plugins.add(p)

        if self.dtb and Path(self.dtb).exists():
            import fdt

            try:
                with open(self.dtb, "rb") as f:
                    dtb = fdt.parse_dtb(f.read())
                    for _path, _nodes, props in dtb.walk():
                        for p in props:
                            if p.name == "compatible":
                                # 'compatible' can be a single string or a list of strings
                                # the fdt library usually returns a list for strings
                                comps = p.data if isinstance(p.data, list) else [p.data]
                                for c in comps:
                                    if not isinstance(c, str):
                                        continue
                                    if c in type_to_plugin:
                                        plugins.add(type_to_plugin[c])
                                    elif c.startswith("virtmcu-"):
                                        plugins.add(c[len("virtmcu-") :])
            except Exception as e:  # noqa: BLE001
                # self.dtb might be a .yaml file or corrupted dtb; fail gracefully.
                # Catching Exception is intentional here as fdt.parse_dtb can throw
                # various undocumented errors on malformed DTBs or YAML input.
                logger.debug(f"Skipping fdt parse of {self.dtb}: {e}")

        plugins.discard("clock")
        return plugins


class Simulation:
    """
    Single SOTA entry point for all firmware-executing simulations.

    Use the `simulation` pytest fixture; do not instantiate directly.
    """

    def __init__(
        self,
        *,
        zenoh_session: zenoh.Session,
        zenoh_router: str,
        qemu_launcher: Callable[..., Coroutine[Any, Any, QmpBridge]],
        init_barrier: bool = True,
    ) -> None:
        self._session = zenoh_session
        self._router = zenoh_router
        self._launcher = qemu_launcher
        self._specs: list[_NodeSpec] = []
        self._bridges: list[QmpBridge] = []
        self._vta: VirtualTimeAuthority | None = None
        # When False, __aenter__ skips vta.init() and ensure_session_routing, wait_for_zenoh_discovery,
        # so the test can drive boot grace-period scenarios. The framework
        # still injects -S and `cont` is still issued at the end. Default True.
        self._init_barrier = init_barrier
        # Optional transport (zenoh / unix / fault-injecting). When set, the VTA
        # is built by the transport so per-transport semantics are honored.
        self.transport: SimulationTransport | None = None

    def add_node(
        self,
        *,
        node_id: int,
        dtb: str | Path,
        kernel: str | Path | None = None,
        extra_args: list[str] | None = None,
        orchestrated: bool = True,
    ) -> None:
        if self._bridges:
            raise RuntimeError("Simulation.add_node() must be called before entering the async context")
        self._specs.append(_NodeSpec(node_id, dtb, kernel, list(extra_args or []), orchestrated))

    @property
    def vta(self) -> VirtualTimeAuthority:
        if self._vta is None:
            raise RuntimeError("Simulation.vta is only available after entering the async context")
        return self._vta

    @property
    def bridge(self) -> QmpBridge:
        if len(self._bridges) != 1:
            raise RuntimeError(f"Simulation.bridge is only valid for single-node sims (have {len(self._bridges)})")
        return self._bridges[0]

    @property
    def bridges(self) -> list[QmpBridge]:
        return list(self._bridges)

    def bridge_for(self, node_id: int) -> QmpBridge:
        """Return the bridge for a specific node_id (registered via `add_node`)."""
        for spec, bridge in zip(self._specs, self._bridges, strict=True):
            if spec.node_id == node_id:
                return bridge
        raise KeyError(f"Simulation has no bridge for node_id={node_id}")

    def uart_buffer(self, node_id: int) -> str:
        """Convenience accessor for guest UART output by node_id."""
        return self.bridge_for(node_id).uart_buffer

    async def __aenter__(self) -> Simulation:
        if not self._specs:
            raise RuntimeError("Simulation has no nodes — call add_node() before entering the context")

        prepared = [self._inject_determinism_args(spec) for spec in self._specs]
        spawn_tasks = [
            self._launcher(
                dtb_path=spec.dtb,
                kernel_path=spec.kernel,
                extra_args=args,
                ignore_clock_check=True,
            )
            for spec, args in zip(self._specs, prepared, strict=True)
        ]
        self._bridges = await asyncio.gather(*spawn_tasks)

        # Only nodes with orchestrated=True (default) participate in the VTA sync loop.
        node_ids = [s.node_id for s in self._specs if s.orchestrated]
        if self.transport is not None:
            self._vta = self.transport.get_vta(node_ids)  # type: ignore[assignment]
        else:
            self._vta = VirtualTimeAuthority(self._session, node_ids)
        assert self._vta is not None

        # When init_barrier=True, we perform the deterministic initialization barrier.
        # If False (e.g. for boot grace-period tests), the test is responsible for
        # calling vta.init() and ensure_session_routing() manually after enter.
        # NOTE: We only wait for plugin liveliness if we are using the Zenoh transport,
        # as plugins skip declaring liveliness tokens on other transports (e.g. Unix sockets).
        if self._init_barrier and node_ids:
            await self._vta.init()
            await ensure_session_routing(self._session)

            is_zenoh = self.transport is None or "Zenoh" in self.transport.__class__.__name__

            if is_zenoh:
                for spec in self._specs:
                    for plugin in spec.plugins:
                        try:
                            await wait_for_zenoh_discovery(
                                self._session, SimTopic.plugin_liveliness(plugin, spec.node_id)
                            )
                        except TimeoutError:
                            logger.warning(
                                f"Timeout waiting for liveliness token of plugin '{plugin}' on node {spec.node_id}"
                            )

        for bridge in self._bridges:
            await bridge.start_emulation()

        return self

    async def __aexit__(self, *exc: object) -> None:
        for bridge in reversed(self._bridges):
            await bridge.close()

    async def run_until(
        self,
        condition: Callable[[], bool],
        *,
        timeout: float = 5.0,
        step_ns: int = 1_000_000,
    ) -> None:
        """
        Advance virtual time in steps of `step_ns` until `condition()` is True
        or `timeout` wall-clock seconds elapse.
        """
        if self._vta is None:
            raise RuntimeError("run_until() called before Simulation context entered")

        start = time.monotonic()
        while time.monotonic() - start < timeout:
            if condition():
                return
            await self._vta.step(step_ns)
        if not condition():
            raise TimeoutError(f"Simulation.run_until: condition not met within {timeout}s")

    def _inject_determinism_args(self, spec: _NodeSpec) -> list[str]:
        """
        Inject standard determinism args into a node's extra_args:
          - `router=`, `node=`, `mode=slaved-icount`, `stall-timeout=` on `virtmcu-clock`
          - `router=` on other `virtmcu` devices/chardevs
          - default `virtmcu-clock` device if none supplied
          - `-icount shift=0,align=off,sleep=off` whenever slaved-icount is in use

        Idempotent: leaves explicitly-supplied flags alone. Mirrors the legacy
        `simulation` fixture's `_create_sim` arg processing so the new lifecycle
        is a drop-in replacement.
        """
        args_in = list(spec.extra_args)
        node_id = spec.node_id
        router = self._router

        base_stall = int(os.environ.get("VIRTMCU_STALL_TIMEOUT_MS", "5000"))
        scaled_stall = int(base_stall * get_time_multiplier())

        processed: list[str] = []
        has_clock = False

        # Mapping from QOM type names / compatible strings to liveliness plugin names
        # Duplicate here for injection logic
        type_to_plugin = {
            "can-host-virtmcu": "canfd",
            "virtmcu": "chardev",
            "virtmcu-chardev": "chardev",
            "netdev": "netdev",
            "spi": "spi",
            "ieee802154": "ieee802154",
            "flexray": "flexray",
            "actuator": "actuator",
            "ui": "ui",
            "telemetry": "telemetry",
            "s32k144-lpuart": "s32k144-lpuart",
        }

        def is_virtmcu_plugin_type(val: str) -> bool:
            type_name = val.split(",")[0]
            return type_name in type_to_plugin or type_name.startswith("virtmcu-")

        i = 0
        while i < len(args_in):
            arg = str(args_in[i])

            # Skip arguments of flags that we know shouldn't be touched
            if arg in ["-serial", "-monitor", "-display", "-cpu", "-m", "-kernel", "-dtb", "-append"]:
                processed.append(arg)
                if i + 1 < len(args_in):
                    processed.append(str(args_in[i + 1]))
                    i += 2
                else:
                    i += 1
                continue

            if arg in ["-device", "-chardev", "-netdev"] and i + 1 < len(args_in):
                val = str(args_in[i + 1])
                if "virtmcu-clock" in val:
                    has_clock = True
                    if "router=" not in val:
                        val = f"{val},router={router}"
                    if "node=" not in val:
                        val = f"{val},node={node_id}"
                    if "mode=" not in val:
                        val = f"{val},mode=slaved-icount"
                    if "stall-timeout=" not in val:
                        val = f"{val},stall-timeout={scaled_stall}"
                elif is_virtmcu_plugin_type(val):
                    if "router=" not in val:
                        val = f"{val},router={router}"
                    if "node=" not in val:
                        val = f"{val},node={node_id}"
                processed.extend([arg, val])
                i += 2
                continue

            if "virtmcu-clock" in arg:
                has_clock = True
                if "router=" not in arg:
                    arg = f"{arg},router={router}"
                if "node=" not in arg:
                    arg = f"{arg},node={node_id}"
                if "mode=" not in arg:
                    arg = f"{arg},mode=slaved-icount"
                if "stall-timeout=" not in arg:
                    arg = f"{arg},stall-timeout={scaled_stall}"
                processed.extend(["-device", arg])
            elif arg == "-global" and i + 1 < len(args_in):
                val = str(args_in[i + 1])
                # We could inject into globals here if needed, but usually they are
                # manually specified in tests that use them.
                processed.extend([arg, val])
                i += 2
                continue
            elif is_virtmcu_plugin_type(arg) and arg not in ["-device", "-chardev", "-global"]:
                if "router=" not in arg:
                    arg = f"{arg},router={router}"
                if "node=" not in arg:
                    arg = f"{arg},node={node_id}"
                prefix = "-chardev" if "id=" in arg else "-device"
                processed.extend([prefix, arg])
            else:
                processed.append(arg)
            i += 1

        if not has_clock:
            processed.extend(
                [
                    "-device",
                    (f"virtmcu-clock,node={node_id},router={router},stall-timeout={scaled_stall},mode=slaved-icount"),
                ]
            )
        if any("slaved-icount" in a for a in processed) and "-icount" not in processed:
            processed.extend(["-icount", "shift=0,align=off,sleep=off"])
        if "-S" not in processed:
            processed.append("-S")
        return processed
