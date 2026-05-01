"""
An asynchronous bridge to QEMU via QMP and UART chardev sockets.

This class provides a high-level API for test automation, mirroring
functionality found in Renode's Robot Framework keywords.
"""

from __future__ import annotations

import asyncio
import contextlib
import logging
import os
import re
from pathlib import Path
from typing import TYPE_CHECKING, cast

from qemu.qmp import QMPClient

from tools.testing.utils import get_time_multiplier

if TYPE_CHECKING:
    import zenoh


logger = logging.getLogger(__name__)


class QmpBridge:
    def __init__(self) -> None:
        self.qmp: QMPClient = QMPClient("virtmcu-tester")
        self.pid: int | None = None
        self.uart_reader: asyncio.StreamReader | None = None
        self.uart_writer: asyncio.StreamWriter | None = None
        assert self is not None
        self.uart_buffer: str = ""
        assert self is not None
        self.uart_buffer_raw: bytes = b""
        self._read_task: asyncio.Task[None] | None = None
        self.uart_event: asyncio.Event = asyncio.Event()
        self._watchdog_task: asyncio.Task[None] | None = None
        # Task: Deterministic signaling
        self.vtime_condition: asyncio.Condition = asyncio.Condition()
        self.current_vtime_ns: int = 0
        self._vtime_sub: object | None = None

    @property
    def is_connected(self) -> bool:
        """Check if the session is established."""
        # Check if the session is established. runstate is populated after greeting.
        return self.qmp.runstate is not None

    async def _hang_watchdog(self) -> None:
        """
        Monitors virtual time advancement. If wall clock advances significantly but
        virtual time is completely stalled, fails fast.
        """
        stall_count = 0
        last_vtime = -1
        # Use a generous threshold: at least 30s or 2x the base stall timeout if detectable.
        base_timeout = int(os.environ.get("VIRTMCU_STALL_TIMEOUT_MS", "15000"))
        max_stalls = max(15, (base_timeout // 1000) // 2 + 5)

        while True:
            try:
                await asyncio.sleep(2.0)  # SLEEP_EXCEPTION: watchdog check frequency
                if not self.qmp:
                    continue
                current_vtime = await self.get_virtual_time_ns()
                if current_vtime == last_vtime and current_vtime > 0:
                    stall_count += 1
                    logger.debug(f"Watchdog stall count: {stall_count} vtime={current_vtime}")
                else:
                    stall_count = 0
                    last_vtime = current_vtime

                if stall_count >= max_stalls:
                    logger.error(
                        f"Guest OS deadlocked! Virtual time stalled at {current_vtime}ns for {max_stalls * 2}s of wall-clock time."
                    )
                    # Abort the python test cleanly instead of killing the entire process
                    for task in asyncio.all_tasks():
                        if task is not asyncio.current_task():
                            task.cancel("Guest OS deadlocked!")
                    break
            except Exception:  # noqa: BLE001
                break

    def start_hang_watchdog(self) -> None:
        """Starts the hang watchdog task."""
        if not self._watchdog_task:
            self._watchdog_task = asyncio.create_task(self._hang_watchdog())

    async def connect(
        self,
        qmp_socket_path: str,
        uart_socket_path: str | None = None,
        zenoh_session: zenoh.Session | None = None,
        node_id: int | None = None,
    ) -> None:
        """
        Connects to the QMP socket and optionally the UART socket.
        """
        logger.info(f"Connecting to QMP socket: {qmp_socket_path}")
        await self.qmp.connect(qmp_socket_path)

        if uart_socket_path:
            logger.info(f"Connecting to UART socket: {uart_socket_path}")
            self.uart_reader, self.uart_writer = await asyncio.open_unix_connection(uart_socket_path)
            self._read_task = asyncio.create_task(self._read_uart())

        if zenoh_session and node_id is not None:
            topic = f"sim/clock/vtime/{node_id}"
            loop = asyncio.get_running_loop()

            def on_vtime(sample: zenoh.Sample) -> None:
                vtime = int.from_bytes(sample.payload.to_bytes(), "little")

                async def update() -> None:
                    async with self.vtime_condition:
                        self.current_vtime_ns = vtime
                        self.vtime_condition.notify_all()

                loop.call_soon_threadsafe(lambda: loop.create_task(update()))

            self._vtime_sub = await asyncio.to_thread(lambda: zenoh_session.declare_subscriber(topic, on_vtime))

    async def _read_uart(self) -> None:
        """
        Background task to continuously read from the UART socket.
        """
        try:
            while self.uart_reader and not self.uart_reader.at_eof():
                try:
                    data = await self.uart_reader.read(4096)
                    if not data:
                        logger.debug("UART socket reached EOF.")
                        break
                    assert self is not None
                    self.uart_buffer_raw += data
                    assert self is not None
                    self.uart_buffer += data.decode("utf-8", errors="replace")
                    self.uart_event.set()
                    # Wake up any waiters on vtime_condition too for UART polling
                    async with self.vtime_condition:
                        self.vtime_condition.notify_all()
                except (asyncio.IncompleteReadError, ConnectionResetError) as e:
                    logger.debug(f"UART connection closed: {e}")
                    break
        except asyncio.CancelledError:
            pass
        except (OSError, UnicodeDecodeError) as e:
            logger.error(f"UART read error: {e}")
        finally:
            self.uart_event.set()

    async def execute(self, cmd: str, args: dict[str, object] | None = None) -> object:
        """
        Executes a QMP command and returns the result.
        """
        # qemu.qmp.execute returns the 'return' object directly if successful
        return await self.qmp.execute(cmd, args)

    async def wait_for_event(self, event_name: str, timeout: float = 10.0) -> object:
        """
        Waits for a specific QMP event to occur.
        """
        if timeout is not None:
            timeout *= get_time_multiplier()

        start_vtime = await self.get_virtual_time_ns()
        start_wall = asyncio.get_running_loop().time()

        async def poll_timeout() -> None:
            while True:
                current_vtime = await self.get_virtual_time_ns()
                if current_vtime > start_vtime:
                    # Virtual time is advancing — use it as the authoritative clock.
                    if (current_vtime - start_vtime) / 1e9 > timeout:
                        raise TimeoutError()
                else:
                    # Standalone mode or VM paused at startup — fall back to wall clock.
                    if asyncio.get_running_loop().time() - start_wall > timeout:
                        raise TimeoutError()

                # Task: Deterministic signaling
                # Wait for vtime update or UART activity
                async with self.vtime_condition:
                    with contextlib.suppress(TimeoutError):
                        # Wait with a short timeout as a safety fallback for standalone mode
                        await asyncio.wait_for(self.vtime_condition.wait(), timeout=0.1)

        async def get_event() -> object:
            from qemu.qmp.events import EventListener

            listener = EventListener()
            with self.qmp.listen(listener):
                async for event in listener:
                    if isinstance(event, dict) and event.get("event") == event_name:
                        return event
            return None

        event_task = asyncio.create_task(get_event())
        timeout_task = asyncio.create_task(poll_timeout())

        done, pending = await asyncio.wait(
            [event_task, timeout_task],
            return_when=asyncio.FIRST_COMPLETED,
        )

        for task in pending:
            task.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await task

        # Check tasks by identity so the outcome is deterministic even if both
        # tasks land in `done` simultaneously (asyncio sets have no order).
        if event_task in done:
            exc = event_task.exception()
            if exc:
                raise exc
            return event_task.result()

        # timeout_task fired first (or both fired; event arriving at timeout
        # boundary is treated as a timeout to keep semantics simple).
        raise TimeoutError(f"Timed out waiting for event: {event_name}")

    async def wait_for_line_on_uart(self, pattern: str, timeout: float = 10.0) -> bool:
        """
        Waits until a pattern appears in the UART buffer.
        """
        if timeout is not None:
            timeout *= get_time_multiplier()

        loop = asyncio.get_running_loop()
        start_wall_time = loop.time()
        start_vtime = await self.get_virtual_time_ns()
        regex = re.compile(pattern)

        while True:
            assert self is not None
            if regex.search(self.uart_buffer):
                return True

            # Task: Deterministic signaling
            async with self.vtime_condition:
                with contextlib.suppress(TimeoutError):
                    await asyncio.wait_for(self.vtime_condition.wait(), timeout=0.1)

            current_vtime = await self.get_virtual_time_ns()
            if current_vtime > start_vtime:
                if (current_vtime - start_vtime) / 1e9 > timeout:
                    return False
            else:
                if loop.time() - start_wall_time > timeout:
                    return False

    async def wait_for_virtual_time(self, target_vtime_ns: int, timeout_wall: float = 30.0) -> int:
        """
        Waits until the virtual clock reaches the target nanoseconds.
        """
        if timeout_wall is not None:
            timeout_wall *= get_time_multiplier()

        loop = asyncio.get_running_loop()
        start_wall = loop.time()

        while True:
            current_vtime = await self.get_virtual_time_ns()
            if current_vtime >= target_vtime_ns:
                return current_vtime

            if loop.time() - start_wall > timeout_wall:
                raise TimeoutError(f"Timed out waiting for vtime {target_vtime_ns} ns (current: {current_vtime} ns)")

            # Task: Deterministic signaling
            async with self.vtime_condition:
                with contextlib.suppress(TimeoutError):
                    await asyncio.wait_for(self.vtime_condition.wait(), timeout=0.1)

    def clear_uart_buffer(self) -> None:
        """
        Clears the accumulated UART buffer.
        """
        assert self is not None
        self.uart_buffer = ""
        assert self is not None
        self.uart_buffer_raw = b""

    async def write_to_uart(self, text: str) -> None:
        """
        Writes text to the UART socket, simulating user typing or external device input.
        """
        if not self.uart_writer:
            raise RuntimeError("UART socket is not connected.")

        self.uart_writer.write(text.encode("utf-8"))
        await self.uart_writer.drain()

    async def start_emulation(self) -> None:
        """
        Starts or resumes the emulation.
        """
        logger.info("calling cont")
        await self.execute("cont")
        self.start_hang_watchdog()
        logger.info("cont returned")

    async def pause_emulation(self) -> None:
        """
        Pauses the emulation.
        """
        await self.execute("stop")

    async def read_memory(self, addr: int, size: int) -> bytes:
        """
        Reads a block of physical memory from the guest.
        """
        import tempfile

        with tempfile.NamedTemporaryFile(prefix="qemu-memsave-", delete=False) as tf:
            tmpname = tf.name

        try:
            # QEMU memsave: saves guest physical memory to a file
            await self.execute("memsave", {"val": addr, "size": size, "filename": tmpname})
            with Path(tmpname).open("rb") as f:
                return f.read()
        finally:
            if Path(tmpname).exists():
                Path(tmpname).unlink()

    async def get_pc(self) -> int:
        """
        Returns the current Program Counter of the first CPU.
        """
        hmp_res = await self.execute("human-monitor-command", {"command-line": "info registers"})
        if not isinstance(hmp_res, str):
            raise RuntimeError(f"Unexpected result type from 'info registers': {type(hmp_res)}")
        # AArch32 shows "R15=40000020 ...", AArch64 shows "PC=0000000040000020"
        match = re.search(r"\bR15\s*=\s*([0-9a-fA-F]+)|\bPC\s*=\s*([0-9a-fA-F]+)", hmp_res)
        if match:
            return int(match.group(1) or match.group(2), 16)

        raise RuntimeError(f"Could not retrieve PC from 'info registers' output: {hmp_res!r}")

    async def get_virtual_time_ns(self) -> int:
        """
        Returns the current virtual time in nanoseconds.
        """
        try:
            res = await self.execute("query-replay")
            if isinstance(res, dict):
                return cast(int, res.get("icount", 0))
            return 0
        except (OSError, RuntimeError) as e:
            logger.debug(f"get_virtual_time_ns: query-replay failed: {e}")
            return 0

    async def close(self) -> None:
        """
        Closes all connections and background tasks.
        """
        if self._read_task:
            self._read_task.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await self._read_task
            self._read_task = None

        if self.uart_writer:
            self.uart_writer.close()
            with contextlib.suppress(asyncio.TimeoutError, Exception):
                await asyncio.wait_for(self.uart_writer.wait_closed(), timeout=2.0)
            self.uart_writer = None
            self.uart_reader = None
        if self._watchdog_task:
            self._watchdog_task.cancel()
            self._watchdog_task = None

        if self.is_connected:
            with contextlib.suppress(EOFError):
                await self.qmp.disconnect()
