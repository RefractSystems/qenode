import asyncio
import time
from collections.abc import Callable
from typing import Any

from tools.testing.qmp_bridge import QmpBridge
from tools.testing.virtmcu_test_suite.conftest_core import VirtualTimeAuthority


class SimNode:
    def __init__(self, node_id: int, bridge: QmpBridge | None):
        self.id = node_id
        self.bridge = bridge

    @property
    def uart(self):
        # QmpBridge has a read_uart_buffer() and wait_for_line() if we need,
        # but for direct buffer inspection we can expose it.
        class UartAccessor:
            def __init__(self, parent):
                self._parent = parent
            @property
            def buffer(self):
                # Returns the accumulated UART bytes from the bridge
                return self._parent.bridge.uart_buffer
        return UartAccessor(self)


class VirtMcuOrchestrator:
    """
    High-level declarative API for multi-node VirtMCU simulations.
    Manages QEMU processes, Zenoh coordinators, and Time Authority clock stepping.
    """
    def __init__(self, zenoh_session, zenoh_router: str, qemu_launcher_fixture):
        self.session = zenoh_session
        self.router = zenoh_router
        self._qemu_launcher = qemu_launcher_fixture
        self._nodes_config: list[dict[str, Any]] = []
        self._nodes: dict[int, SimNode] = {}
        self.vta: VirtualTimeAuthority | None = None
        self._vtime_ns: int = 0

    def add_node(self, node_id: int, dtb_path: str, kernel_path: str, extra_args: list[str] | None = None) -> SimNode:
        if extra_args is None:
            extra_args = []

        # Automatically setup determinism if not already provided
        # It's better if we check if clock is already there, but we can assume Orchestrator owns it
        has_clock = any("clock" in str(arg) for arg in extra_args)
        if not has_clock:
            extra_args.extend(
                [
                    "-icount",
                    "shift=0,align=off,sleep=off",
                    "-device",
                    f"virtmcu-clock,mode=slaved-icount,node={node_id},router={self.router}",
                ]
            )

        self._nodes_config.append(
            {
                "id": node_id,
                "dtb_path": dtb_path,
                "kernel_path": kernel_path,
                "extra_args": extra_args,
            }
        )

        node = SimNode(node_id, None)
        self._nodes[node_id] = node
        return node

    async def __aenter__(self):
        return self

    async def start(self):
        node_ids = []
        tasks = []
        for config in self._nodes_config:
            node_ids.append(config["id"])
            tasks.append(
                self._qemu_launcher(
                    dtb_path=config["dtb_path"],
                    kernel_path=config["kernel_path"],
                    extra_args=config["extra_args"],
                    ignore_clock_check=True,
                )
            )

        bridges = await asyncio.gather(*tasks)
        for config, bridge in zip(self._nodes_config, bridges, strict=True):
            self._nodes[config["id"]].bridge = bridge

        self.vta = VirtualTimeAuthority(self.session, node_ids)

    async def run_until(self, condition: Callable[[], bool], timeout: float = 5.0, step_ns: int = 1_000_000):
        """
        Advances the simulation clock in steps of `step_ns` until `condition()` is True
        or `timeout` seconds of wall-clock time elapse.
        """
        start_time = time.time()
        while time.time() - start_time < timeout:
            if condition():
                return
            if self.vta:
                await self.vta.step(step_ns)
            self._vtime_ns += step_ns
            await asyncio.sleep(0.001)  # SLEEP_EXCEPTION: deliberate yielding

        if not condition():
            raise TimeoutError(f"Condition not met within {timeout}s. Current vtime: {self._vtime_ns}ns")

    async def __aexit__(self, exc_type, exc_val, exc_tb):
        # qemu_launcher automatically registers processes with an AsyncManagedProcess or similar cleanup
        # within its own fixture scope (it's using an async generator in conftest).
        pass
