import asyncio
import time
from collections.abc import Callable
from typing import Any

from tools.testing.qmp_bridge import QmpBridge
from tools.testing.utils import yield_now
from tools.testing.virtmcu_test_suite.conftest_core import VirtualTimeAuthority
from tools.testing.virtmcu_test_suite.transport import SimulationTransport


class SimNode:
    def __init__(self, node_id: int, bridge: QmpBridge | None) -> None:
        self.id = node_id
        self.bridge = bridge

    @property
    def uart(self) -> Any:
        # QmpBridge has a read_uart_buffer() and wait_for_line() if we need,
        # but for direct buffer inspection we can expose it.
        class UartAccessor:
            def __init__(self, parent: "SimNode") -> None:
                self._parent = parent

            @property
            def buffer(self) -> str:
                # Returns the accumulated UART bytes from the bridge
                if self._parent.bridge is None:
                    return ""
                return self._parent.bridge.uart_buffer

        return UartAccessor(self)


class SimulationOrchestrator:
    """
    High-level declarative API for multi-node VirtMCU simulations.
    Manages QEMU processes, Zenoh coordinators, and Time Authority clock stepping.
    """

    def __init__(self, zenoh_session: Any, zenoh_router: str, qemu_launcher_fixture: Any) -> None:
        self.session = zenoh_session
        self.router = zenoh_router
        self._qemu_launcher = qemu_launcher_fixture
        self._nodes_config: list[dict[str, Any]] = []
        self._nodes: dict[int, SimNode] = {}
        self.vta: VirtualTimeAuthority | None = None
        self._vtime_ns: int = 0
        self.transport: SimulationTransport | None = None

    def add_node(self, node_id: int, dtb_path: str, kernel_path: str, extra_args: list[str] | None = None) -> SimNode:
        if extra_args is None:
            extra_args = []

        # Automatically setup determinism if not already provided
        # It's better if we check if clock is already there, but we can assume Orchestrator owns it
        has_clock = any("clock" in str(arg) for arg in extra_args)
        if not has_clock:
            extra_args.extend(["-icount", "shift=0,align=off,sleep=off"])
            if self.transport:
                extra_args.extend(["-device", self.transport.get_clock_device_str(node_id)])
            else:
                extra_args.extend(
                    [
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

    async def __aenter__(self) -> "SimulationOrchestrator":
        return self

    async def start(self) -> None:
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
            await bridge.start_emulation()

        if self.transport:
            self.vta = self.transport.get_vta(node_ids)
        else:
            self.vta = VirtualTimeAuthority(self.session, node_ids)
        await self.vta.init()

    async def run_until(self, condition: Callable[[], bool], timeout: float = 5.0, step_ns: int = 1_000_000) -> None:
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
            await yield_now()

        if not condition():
            raise TimeoutError(f"Condition not met within {timeout}s. Current vtime: {self._vtime_ns}ns")

    async def __aexit__(self, exc_type: Any, exc_val: Any, exc_tb: Any) -> None:
        # qemu_launcher automatically registers processes with an AsyncManagedProcess or similar cleanup
        # within its own fixture scope (it's using an async generator in conftest).
        pass
