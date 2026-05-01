import abc
import asyncio
import logging
import random
import shutil
import socket
import tempfile
from collections.abc import Callable
from pathlib import Path
from typing import TYPE_CHECKING, Any

import vproto

if TYPE_CHECKING:
    import zenoh

logger = logging.getLogger(__name__)


class SimulationTransport(abc.ABC):
    @abc.abstractmethod
    def dump_flight_recorder(self) -> list[dict]: ...
    @abc.abstractmethod
    def dump_pcap(self, path: Path) -> None: ...

    @abc.abstractmethod
    async def start(self) -> None: ...

    @abc.abstractmethod
    async def stop(self) -> None: ...

    @abc.abstractmethod
    def get_clock_device_str(self, node_id: int) -> str: ...

    @abc.abstractmethod
    def get_peripheral_props(self) -> str: ...

    @abc.abstractmethod
    def dtb_router_endpoint(self) -> str: ...

    @abc.abstractmethod
    async def publish(self, topic: str, payload: bytes) -> None: ...

    @abc.abstractmethod
    async def subscribe(self, topic: str, callback: Callable[[bytes], None]) -> None: ...

    @abc.abstractmethod
    async def step_clock(self, delta_ns: int) -> tuple[int, int]: ...

    @abc.abstractmethod
    def get_vta(self, node_ids: list[int]) -> Any: ...


class ZenohTransportImpl(SimulationTransport):
    def dump_flight_recorder(self) -> list[dict]:
        return getattr(self, "history", [])

    def dump_pcap(self, path: Path) -> None:
        from tools.testing.virtmcu_test_suite.pcap_writer import write_pcap

        write_pcap(path, self.history)

    def __init__(self, router_endpoint: str, session: "zenoh.Session") -> None:
        self.router_endpoint = router_endpoint
        self.session = session
        self.subs: list[zenoh.Subscriber] = []
        from tools.testing.virtmcu_test_suite.conftest_core import VirtualTimeAuthority

        self.vta = VirtualTimeAuthority(session, [0])  # Assumes single node 0 for basic tests
        self.history: list[dict] = []

    def get_vta(self, node_ids: list[int]) -> Any:
        from tools.testing.virtmcu_test_suite.conftest_core import VirtualTimeAuthority

        return VirtualTimeAuthority(self.session, node_ids)

    async def start(self) -> None:
        pass

    async def stop(self) -> None:
        for sub in self.subs:
            await asyncio.to_thread(sub.undeclare)

    def get_clock_device_str(self, node_id: int) -> str:
        return f"virtmcu-clock,mode=slaved-icount,node={node_id},router={self.router_endpoint}"

    def get_peripheral_props(self) -> str:
        return f"router={self.router_endpoint}"

    def dtb_router_endpoint(self) -> str:
        return self.router_endpoint

    async def publish(self, topic: str, payload: bytes) -> None:
        import time

        vtime = self.vta.current_vtimes.get(0, 0) if hasattr(self.vta, "current_vtimes") else 0
        self.history.append(
            {"time": time.time(), "vtime_ns": vtime, "topic": topic, "payload": payload.hex(), "direction": "tx"}
        )
        await asyncio.to_thread(lambda: self.session.put(topic, payload))

    async def subscribe(self, topic: str, callback: Callable[[bytes], None]) -> None:
        import time

        loop = asyncio.get_running_loop()

        def _cb(sample: "zenoh.Sample") -> None:
            p = sample.payload.to_bytes()

            # Offload to loop thread to avoid Zenoh C-thread deadlock
            def run_in_loop():
                vtime = self.vta.current_vtimes.get(0, 0) if hasattr(self.vta, "current_vtimes") else 0
                self.history.append(
                    {"time": time.time(), "vtime_ns": vtime, "topic": topic, "payload": p.hex(), "direction": "rx"}
                )
                callback(p)

            loop.call_soon_threadsafe(run_in_loop)

        sub = await asyncio.to_thread(lambda: self.session.declare_subscriber(topic, _cb))
        self.subs.append(sub)

    async def step_clock(self, delta_ns: int) -> tuple[int, int]:
        return await self.vta.step(delta_ns)


class UnixTransportImpl(SimulationTransport):
    def dump_flight_recorder(self) -> list[dict]:
        return getattr(self, "history", [])

    def dump_pcap(self, path: Path) -> None:
        from tools.testing.virtmcu_test_suite.pcap_writer import write_pcap

        write_pcap(path, self.history)

    def __init__(self) -> None:
        self.tmpdir = tempfile.mkdtemp(prefix="virtmcu-unix-transport-")
        self.clock_sock = str(Path(self.tmpdir) / "clock.sock")
        self.data_sock = str(Path(self.tmpdir) / "data.sock")

        self.clock_server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.clock_server.bind(self.clock_sock)
        self.clock_server.listen(1)
        self.clock_conn: socket.socket | None = None

        self.data_subs: list[tuple[str, Callable[[bytes], None]]] = []
        self.data_conns: list[asyncio.StreamWriter] = []
        self._data_server_task = None
        self._clock_accept_task: asyncio.Task | None = None
        self.vtime_ns = 0
        self.history: list[dict] = []

    async def start(self) -> None:
        self.server = await asyncio.start_unix_server(self._handle_data_conn, self.data_sock)

        loop = asyncio.get_running_loop()
        self.clock_server.setblocking(False)

        async def _accept_clock() -> None:
            self.clock_conn, _ = await loop.sock_accept(self.clock_server)

        self._clock_accept_task = asyncio.create_task(_accept_clock())

    async def stop(self) -> None:
        if self._clock_accept_task:
            self._clock_accept_task.cancel()
        if self.clock_conn:
            self.clock_conn.close()
        self.clock_server.close()

        self.server.close()
        await self.server.wait_closed()
        shutil.rmtree(self.tmpdir, ignore_errors=True)

    async def _handle_data_conn(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        self.data_conns.append(writer)
        try:
            while True:
                topic_len_b = await reader.readexactly(4)
                topic_len = int.from_bytes(topic_len_b, "little")
                topic = (await reader.readexactly(topic_len)).decode()

                payload_len_b = await reader.readexactly(4)
                payload_len = int.from_bytes(payload_len_b, "little")
                payload = await reader.readexactly(payload_len)
                logger.info(f"UnixTransportImpl rx: {topic}")

                import time

                self.history.append(
                    {
                        "time": time.time(),
                        "vtime_ns": self.vtime_ns,
                        "topic": topic,
                        "payload": payload.hex(),
                        "direction": "rx",
                    }
                )
                for sub_topic, cb in self.data_subs:
                    if topic == sub_topic or topic.startswith(sub_topic):
                        cb(payload)
        except asyncio.IncompleteReadError:
            pass
        finally:
            self.data_conns.remove(writer)

    def get_clock_device_str(self, node_id: int) -> str:
        return f"virtmcu-clock,mode=slaved-unix,node={node_id},router={self.clock_sock}"

    def get_peripheral_props(self) -> str:
        return f"transport=unix,router={self.data_sock}"

    def dtb_router_endpoint(self) -> str:
        return self.data_sock  # Unix sockets don't use TCP endpoints in DTB for standalone run

    async def publish(self, topic: str, payload: bytes) -> None:
        msg = len(topic).to_bytes(4, "little") + topic.encode() + len(payload).to_bytes(4, "little") + payload
        import time

        self.history.append(
            {
                "time": time.time(),
                "vtime_ns": self.vtime_ns,
                "topic": topic,
                "payload": payload.hex(),
                "direction": "tx",
            }
        )
        for w in self.data_conns:
            w.write(msg)
            await w.drain()

        # Simulate loopback for local tests
        for sub_topic, cb in self.data_subs:
            if topic == sub_topic or topic.startswith(sub_topic):
                cb(payload)

    async def subscribe(self, topic: str, callback: Callable[[bytes], None]) -> None:
        self.data_subs.append((topic, callback))

    async def step_clock(self, delta_ns: int) -> tuple[int, int]:
        if not self.clock_conn:
            assert self._clock_accept_task is not None
            await self._clock_accept_task
            self._clock_accept_task = None

        assert self.clock_conn is not None
        req = vproto.ClockAdvanceReq(delta_ns, self.vtime_ns + delta_ns, 0).pack()
        loop = asyncio.get_running_loop()
        await loop.sock_sendall(self.clock_conn, req)

        resp_data = b""
        while len(resp_data) < 24:
            chunk = await loop.sock_recv(self.clock_conn, 24 - len(resp_data))
            if not chunk:
                raise RuntimeError("Clock connection closed")
            resp_data += chunk

        resp = vproto.ClockReadyResp.unpack(resp_data)
        if resp.error_code != 0:
            raise RuntimeError(f"Clock stall error: {resp.error_code}")
        self.vtime_ns = resp.current_vtime_ns
        return resp.current_vtime_ns, resp.quantum_number

    def get_vta(self, node_ids: list[int]) -> Any:
        return UnixVirtualTimeAuthority(self, node_ids)


class UnixVirtualTimeAuthority:
    def __init__(self, transport: UnixTransportImpl, node_ids: list[int]):
        self.transport = transport
        self.node_ids = node_ids
        self.current_vtimes = dict.fromkeys(node_ids, 0)

    async def init(self, _timeout: float = 30.0):
        # QEMU connects to the socket when it's ready.
        # UnixTransportImpl.step_clock will wait for the connection.
        await self.step(0)

    async def step(self, delta_ns: int, _timeout: float | None = None):
        # Note: UnixTransportImpl currently only supports a single connection/node.
        vtime, _ = await self.transport.step_clock(delta_ns)
        for nid in self.node_ids:
            self.current_vtimes[nid] = vtime
        return self.current_vtimes


class FaultInjectingTransport(SimulationTransport):
    """
    DET-3: Wraps an existing SimulationTransport to introduce artificial
    packet loss and latency (Chaos Engineering) for deterministic robustness testing.
    """

    def __init__(
        self,
        inner: SimulationTransport,
        drop_prob: float = 0.0,
        delay_s: float = 0.0,
        jitter_s: float = 0.0,
    ) -> None:
        self.inner = inner
        self.drop_prob = drop_prob
        self.delay_s = delay_s
        self.jitter_s = jitter_s
        self._tasks: set[asyncio.Task] = set()

    def get_vta(self, node_ids: list[int]) -> Any:
        return self.inner.get_vta(node_ids)

    async def start(self) -> None:
        await self.inner.start()

    async def stop(self) -> None:
        await self.inner.stop()

    def get_clock_device_str(self, node_id: int) -> str:
        return self.inner.get_clock_device_str(node_id)

    def get_peripheral_props(self) -> str:
        return self.inner.get_peripheral_props()

    def dtb_router_endpoint(self) -> str:
        return self.inner.dtb_router_endpoint()

    def _should_drop(self, payload: bytes) -> bool:
        if payload in (b"ping", b"sync"):
            return False
        return self.drop_prob > 0.0 and random.random() < self.drop_prob

    def _get_delay(self, payload: bytes) -> float:
        if payload in (b"ping", b"sync"):
            return 0.0
        d = self.delay_s
        if self.jitter_s > 0.0:
            d += (random.random() * 2 - 1) * self.jitter_s
        return max(0.0, d)

    def _get_vtime_ns(self) -> int:
        if hasattr(self.inner, "vta") and self.inner.vta:
            if hasattr(self.inner.vta, "current_vtimes"):
                return self.inner.vta.current_vtimes.get(0, 0)
            if hasattr(self.inner.vta, "current_vtime_ns"):
                return self.inner.vta.current_vtime_ns
        elif hasattr(self.inner, "vtime_ns"):
            return self.inner.vtime_ns
        return 0

    async def publish(self, topic: str, payload: bytes) -> None:
        if self._should_drop(payload):
            # Deliberately drop packet
            if hasattr(self.inner, "history"):
                self.inner.history.append(
                    {
                        "time": __import__("time").time(),
                        "vtime_ns": self._get_vtime_ns(),
                        "topic": topic,
                        "payload": payload.hex(),
                        "direction": "tx_dropped",
                    }
                )
            return

        delay = self._get_delay(payload)
        if delay > 0.0:
            await asyncio.sleep(delay)  # SLEEP_EXCEPTION: Chaos Engineering delay injection

        await self.inner.publish(topic, payload)

    async def subscribe(self, topic: str, callback: Callable[[bytes], None]) -> None:
        async def wrapped_callback(payload: bytes) -> None:
            if self._should_drop(payload):
                if hasattr(self.inner, "history"):
                    self.inner.history.append(
                        {
                            "time": __import__("time").time(),
                            "vtime_ns": self._get_vtime_ns(),
                            "topic": topic,
                            "payload": payload.hex(),
                            "direction": "rx_dropped",
                        }
                    )
                return

            delay = self._get_delay(payload)
            if delay > 0.0:
                await asyncio.sleep(delay)  # SLEEP_EXCEPTION: Chaos Engineering delay injection

            if asyncio.iscoroutinefunction(callback):
                await callback(payload)
            else:
                callback(payload)

        # underlying transport (like Zenoh) must call thread_safe_callback in the loop thread
        def thread_safe_callback(payload: bytes) -> None:
            try:
                loop = asyncio.get_running_loop()
                if loop.is_running():
                    task = loop.create_task(wrapped_callback(payload))
                    self._tasks.add(task)
                    task.add_done_callback(self._tasks.discard)
                else:
                    pass
            except RuntimeError:
                pass

        await self.inner.subscribe(topic, thread_safe_callback)

    async def step_clock(self, delta_ns: int) -> tuple[int, int]:
        return await self.inner.step_clock(delta_ns)

    def dump_flight_recorder(self) -> list[dict]:
        return getattr(self.inner, "dump_flight_recorder", lambda: [])()

    def dump_pcap(self, path: Path) -> None:
        if hasattr(self.inner, "dump_pcap"):
            self.inner.dump_pcap(path)
