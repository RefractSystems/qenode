import asyncio
import logging
import re
from typing import Any, Dict, Optional

from qemu.qmp import QMPClient
from qemu.qmp.protocol import Runstate

logger = logging.getLogger(__name__)


class QmpBridge:
    """
    An asynchronous bridge to QEMU via QMP and UART chardev sockets.

    This class provides a high-level API for test automation, mirroring
    functionality found in Renode's Robot Framework keywords.
    """

    def __init__(self):
        self.qmp = QMPClient("virtmcu-tester")
        self.uart_reader: Optional[asyncio.StreamReader] = None
        self.uart_writer: Optional[asyncio.StreamWriter] = None
        self.uart_buffer = ""
        self._read_task: Optional[asyncio.Task] = None

    @property
    def is_connected(self) -> bool:
        return self.qmp.runstate == Runstate.RUNNING

    async def connect(self, qmp_socket_path: str, uart_socket_path: Optional[str] = None):
        """
        Connects to the QMP socket and optionally the UART socket.
        """
        logger.info(f"Connecting to QMP socket: {qmp_socket_path}")
        await self.qmp.connect(qmp_socket_path)

        if uart_socket_path:
            logger.info(f"Connecting to UART socket: {uart_socket_path}")
            self.uart_reader, self.uart_writer = await asyncio.open_unix_connection(uart_socket_path)
            self._read_task = asyncio.create_task(self._read_uart())

    async def _read_uart(self):
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
                    self.uart_buffer += data.decode("utf-8", errors="replace")
                except (asyncio.IncompleteReadError, ConnectionResetError) as e:
                    logger.debug(f"UART connection closed: {e}")
                    break
        except asyncio.CancelledError:
            pass
        except Exception as e:
            logger.error(f"UART read error: {e}")

    async def execute(self, cmd: str, args: Optional[Dict[str, Any]] = None) -> Any:
        """
        Executes a QMP command and returns the result.
        """
        if not self.is_connected:
             raise RuntimeError("QMP is not connected.")
        # qemu.qmp.execute returns the 'return' object directly if successful
        return await self.qmp.execute(cmd, args)

    async def wait_for_event(self, event_name: str, timeout: float = 10.0) -> Any:
        """
        Waits for a specific QMP event to occur.

        Uses virtual time (instruction count via query-replay) when the VM is
        running in slaved-icount mode so that CI tests do not time out
        prematurely under heavy co-simulation load.  Falls back to wall-clock
        time in standalone mode where virtual time is not advancing.
        """
        start_vtime = await self.get_virtual_time_ns()
        start_wall = asyncio.get_running_loop().time()

        async def poll_timeout():
            while True:
                current_vtime = await self.get_virtual_time_ns()
                if current_vtime > start_vtime:
                    # Virtual time is advancing — use it as the authoritative clock.
                    if (current_vtime - start_vtime) / 1e9 > timeout:
                        raise asyncio.TimeoutError()
                else:
                    # Standalone mode or VM paused at startup — fall back to wall clock.
                    if asyncio.get_running_loop().time() - start_wall > timeout:
                        raise asyncio.TimeoutError()
                await asyncio.sleep(0.1)

        async def get_event():
            async with self.qmp.listen() as listener:
                async for event in listener:
                    if event["event"] == event_name:
                        return event

        event_task = asyncio.create_task(get_event())
        timeout_task = asyncio.create_task(poll_timeout())

        done, pending = await asyncio.wait(
            [event_task, timeout_task],
            return_when=asyncio.FIRST_COMPLETED,
        )

        for task in pending:
            task.cancel()
            try:
                await task
            except asyncio.CancelledError:
                pass

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

        Returns True on match, False on timeout.  Uses virtual time when the VM
        runs in slaved-icount mode; falls back to wall clock in standalone mode.
        """
        loop = asyncio.get_running_loop()
        start_wall_time = loop.time()
        start_vtime = await self.get_virtual_time_ns()
        regex = re.compile(pattern)

        while True:
            if regex.search(self.uart_buffer):
                return True

            current_vtime = await self.get_virtual_time_ns()
            if current_vtime > start_vtime:
                # Virtual time is advancing — rely on it strictly.
                if (current_vtime - start_vtime) / 1e9 > timeout:
                    return False
            else:
                # Fallback: virtual time stuck at 0 (standalone mode or early boot).
                if loop.time() - start_wall_time > timeout:
                    return False

            await asyncio.sleep(0.1)

    def clear_uart_buffer(self):
        """
        Clears the accumulated UART buffer.
        """
        self.uart_buffer = ""

    async def write_to_uart(self, text: str):
        """
        Writes text to the UART socket, simulating user typing or external device input.
        """
        if not self.uart_writer:
            raise RuntimeError("UART socket is not connected.")

        self.uart_writer.write(text.encode("utf-8"))
        await self.uart_writer.drain()

    async def start_emulation(self):
        """
        Starts or resumes the emulation.
        """
        await self.execute("cont")

    async def pause_emulation(self):
        """
        Pauses the emulation.
        """
        await self.execute("stop")

    async def get_pc(self) -> int:
        """
        Returns the current Program Counter of the first CPU.

        query-cpus-fast (CpuInfoFast) does not expose register values — it only
        carries cpu-index, qom-path, thread-id, and target-arch. We read PC via
        HMP 'info registers', which works for all ARM variants (AArch32: R15,
        AArch64: PC).
        """
        hmp_res = await self.execute("human-monitor-command", {"command-line": "info registers"})
        # AArch32 shows "R15=40000020 ...", AArch64 shows "PC=0000000040000020"
        match = re.search(r"\bR15\s*=\s*([0-9a-fA-F]+)|\bPC\s*=\s*([0-9a-fA-F]+)", hmp_res)
        if match:
            return int(match.group(1) or match.group(2), 16)

        raise RuntimeError(f"Could not retrieve PC from 'info registers' output: {hmp_res!r}")

    async def get_virtual_time_ns(self) -> int:
        """
        Returns the current virtual time in nanoseconds.

        Uses the ``query-replay`` QMP command, which returns a ``ReplayInfo``
        struct whose ``icount`` field is the current instruction count.  In
        slaved-icount mode (``-icount shift=0,align=off,sleep=off``), QEMU
        increments icount by exactly 1 per instruction and each instruction
        represents 1 virtual nanosecond, so icount == virtual_time_ns.

        In standalone mode (no ``-icount``), ``query-replay`` still succeeds
        and returns ``mode: "none"`` with ``icount: 0``, allowing callers to
        detect that virtual time is not advancing and fall back to wall-clock
        time (see ``wait_for_line_on_uart`` and ``wait_for_event``).

        Returns 0 on any QMP error (e.g. QEMU not yet fully initialised).
        """
        try:
            res = await self.execute("query-replay")
            return res.get("icount", 0)
        except Exception as e:
            logger.debug(f"get_virtual_time_ns: query-replay failed: {e}")
            return 0

    async def close(self):
        """
        Closes all connections and background tasks.
        """
        if self._read_task:
            self._read_task.cancel()
            try:
                await self._read_task
            except asyncio.CancelledError:
                pass
            self._read_task = None

        if self.uart_writer:
            self.uart_writer.close()
            try:
                await asyncio.wait_for(self.uart_writer.wait_closed(), timeout=2.0)
            except (asyncio.TimeoutError, Exception):
                pass
            self.uart_writer = None
            self.uart_reader = None

        if self.is_connected:
            await self.qmp.disconnect()

