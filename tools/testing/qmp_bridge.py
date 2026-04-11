import asyncio
import logging
import re
from typing import Any, Dict, Optional

from qemu.qmp import QMPClient

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
                data = await self.uart_reader.read(4096)
                if not data:
                    break
                self.uart_buffer += data.decode("utf-8", errors="replace")
        except asyncio.CancelledError:
            pass
        except Exception as e:
            logger.error(f"UART read error: {e}")

    async def execute(self, cmd: str, args: Optional[Dict[str, Any]] = None) -> Any:
        """
        Executes a QMP command and returns the result.
        """
        # qemu.qmp.execute returns the 'return' object directly if successful
        return await self.qmp.execute(cmd, args)

    async def wait_for_event(self, event_name: str, timeout: float = 10.0) -> Any:
        """
        Waits for a specific QMP event to occur.
        """
        try:
            async with self.qmp.listen() as listener:
                return await asyncio.wait_for(self._find_event(listener, event_name), timeout)
        except asyncio.TimeoutError:
            raise TimeoutError(f"Timed out waiting for event: {event_name}")

    async def _find_event(self, listener, event_name):
        async for event in listener:
            if event["event"] == event_name:
                return event

    async def wait_for_line_on_uart(self, pattern: str, timeout: float = 10.0) -> bool:
        """
        Waits until a pattern appears in the UART buffer.
        Returns True on match, False on timeout.
        """
        loop = asyncio.get_running_loop()
        start_time = loop.time()
        regex = re.compile(pattern)
        while loop.time() - start_time < timeout:
            if regex.search(self.uart_buffer):
                return True
            await asyncio.sleep(0.1)
        return False

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

    def get_virtual_time_ns(self) -> int:
        """
        Returns the current virtual time in nanoseconds.

        In standalone mode (Phase 4), this might return 0 or be un-synced.
        Full implementation is deferred to Phase 7.
        """
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

        if self.uart_writer:
            self.uart_writer.close()
            await self.uart_writer.wait_closed()

        await self.qmp.disconnect()
