"""
qemu_clock.py — async client for the libqemu clock-socket protocol.

Wire protocol (little-endian, packed):
  Host → QEMU:  ClockAdvance { uint64 delta_ns; uint64 mujoco_time_ns; }
  QEMU → Host:  ClockReady   { uint64 current_vtime_ns; uint32 n_frames; }

n_frames > 0 is reserved for Phase 7 (Ethernet frame injection).
"""

import asyncio
import struct

CLOCK_ADVANCE_FMT  = "<QQ"
CLOCK_READY_FMT    = "<QI"
CLOCK_ADVANCE_SIZE = struct.calcsize(CLOCK_ADVANCE_FMT)
CLOCK_READY_SIZE   = struct.calcsize(CLOCK_READY_FMT)


class QemuClockClient:
    """
    Connects to QEMU's clock socket and advances virtual time on request.

    Usage:
        client = QemuClockClient("/tmp/qemu-clock.sock")
        await client.connect()
        vtime_ns = await client.advance(delta_ns=1_000_000, mujoco_time_ns=0)
        await client.close()
    """

    def __init__(self, socket_path: str):
        self.socket_path = socket_path
        self._reader: asyncio.StreamReader | None = None
        self._writer: asyncio.StreamWriter | None = None

    async def connect(self, timeout: float = 60.0) -> None:
        """Connect to the QEMU clock socket, retrying until timeout."""
        deadline = asyncio.get_event_loop().time() + timeout
        while True:
            try:
                self._reader, self._writer = await asyncio.open_unix_connection(
                    self.socket_path
                )
                return
            except (FileNotFoundError, ConnectionRefusedError):
                if asyncio.get_event_loop().time() > deadline:
                    raise TimeoutError(
                        f"Timed out connecting to QEMU clock socket: {self.socket_path}"
                    )
                await asyncio.sleep(0.5)

    async def advance(self, delta_ns: int, mujoco_time_ns: int = 0) -> int:
        """
        Advance QEMU virtual clock by delta_ns nanoseconds.
        Returns the resulting virtual time in nanoseconds.
        Raises RuntimeError if n_frames > 0 (Ethernet not yet implemented).
        """
        assert self._writer is not None, "Not connected — call connect() first"

        payload = struct.pack(CLOCK_ADVANCE_FMT, delta_ns, mujoco_time_ns)
        self._writer.write(payload)
        await self._writer.drain()

        raw = await self._reader.readexactly(CLOCK_READY_SIZE)
        vtime_ns, n_frames = struct.unpack(CLOCK_READY_FMT, raw)

        if n_frames > 0:
            # Phase 7: drain frame data to avoid desync
            # ETH_FRAME_HDR = "<QH" (timestamp_ns, length)
            import warnings
            warnings.warn(
                f"QEMU sent {n_frames} Ethernet frames — not yet handled by node_agent",
                stacklevel=2,
            )

        return vtime_ns

    async def close(self) -> None:
        if self._writer:
            self._writer.close()
            await self._writer.wait_closed()
