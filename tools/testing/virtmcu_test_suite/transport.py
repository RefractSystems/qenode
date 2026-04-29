import abc
import asyncio
import logging
import random
import shutil
import socket
import tempfile
from collections.abc import Callable
from pathlib import Path

import vproto

logger = logging.getLogger(__name__)


class SimulationTransport(abc.ABC):
    @abc.abstractmethod
    def dump_flight_recorder(self) -> list[dict]: ...
    @abc.abstractmethod
    async def start(self): ...

    @abc.abstractmethod
    async def stop(self): ...

    @abc.abstractmethod
    def get_clock_device_str(self, node_id: int) -> str: ...

    @abc.abstractmethod
    def get_peripheral_props(self) -> str: ...

    @abc.abstractmethod
    def dtb_router_endpoint(self) -> str: ...

    @abc.abstractmethod
    async def publish(self, topic: str, payload: bytes): ...

    @abc.abstractmethod
    async def subscribe(self, topic: str, callback: Callable[[bytes], None]): ...

    @abc.abstractmethod
    async def step_clock(self, delta_ns: int): ...


class ZenohTransportImpl(SimulationTransport):
    def dump_flight_recorder(self) -> list[dict]:
        return getattr(self, "history", [])

    def __init__(self, router_endpoint, session):
        self.router_endpoint = router_endpoint
        self.session = session
        self.subs = []
        from tools.testing.virtmcu_test_suite.conftest_core import VirtualTimeAuthority

        self.vta = VirtualTimeAuthority(session, [0])  # Assumes single node 0 for basic tests
        self.history = []

    async def start(self):
        pass

    async def stop(self):
        for sub in self.subs:
            await asyncio.to_thread(sub.undeclare)

    def get_clock_device_str(self, node_id: int) -> str:
        return f"virtmcu-clock,mode=slaved-icount,node={node_id},router={self.router_endpoint}"

    def get_peripheral_props(self) -> str:
        return f"router={self.router_endpoint}"

    def dtb_router_endpoint(self) -> str:
        return self.router_endpoint

    async def publish(self, topic: str, payload: bytes):
        import time

        self.history.append({"time": time.time(), "topic": topic, "payload": payload.hex(), "direction": "tx"})
        await asyncio.to_thread(lambda: self.session.put(topic, payload))

    async def subscribe(self, topic: str, callback: Callable[[bytes], None]):
        import time

        def _cb(sample):
            p = sample.payload.to_bytes()
            self.history.append({"time": time.time(), "topic": topic, "payload": p.hex(), "direction": "rx"})
            callback(p)

        sub = await asyncio.to_thread(lambda: self.session.declare_subscriber(topic, _cb))
        self.subs.append(sub)

    async def step_clock(self, delta_ns: int):
        await self.vta.step(delta_ns)


class UnixTransportImpl(SimulationTransport):
    def dump_flight_recorder(self) -> list[dict]:
        return getattr(self, "history", [])

    def __init__(self):
        self.tmpdir = tempfile.mkdtemp(prefix="virtmcu-unix-transport-")
        self.clock_sock = str(Path(self.tmpdir) / "clock.sock")
        self.data_sock = str(Path(self.tmpdir) / "data.sock")

        self.clock_server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.clock_server.bind(self.clock_sock)
        self.clock_server.listen(1)
        self.clock_conn: socket.socket | None = None

        self.data_subs = []
        self.data_conns = []
        self._data_server_task = None
        self._clock_accept_task: asyncio.Task | None = None
        self.vtime_ns = 0
        self.history = []

    async def start(self):
        self.server = await asyncio.start_unix_server(self._handle_data_conn, self.data_sock)

        loop = asyncio.get_running_loop()
        self.clock_server.setblocking(False)

        async def _accept_clock():
            self.clock_conn, _ = await loop.sock_accept(self.clock_server)

        self._clock_accept_task = asyncio.create_task(_accept_clock())

    async def stop(self):
        if self._clock_accept_task:
            self._clock_accept_task.cancel()
        if self.clock_conn:
            self.clock_conn.close()
        self.clock_server.close()

        self.server.close()
        await self.server.wait_closed()
        shutil.rmtree(self.tmpdir, ignore_errors=True)

    async def _handle_data_conn(self, reader, writer):
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

                self.history.append({"time": time.time(), "topic": topic, "payload": payload.hex(), "direction": "rx"})
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

    async def publish(self, topic: str, payload: bytes):
        msg = len(topic).to_bytes(4, "little") + topic.encode() + len(payload).to_bytes(4, "little") + payload
        import time

        self.history.append({"time": time.time(), "topic": topic, "payload": payload.hex(), "direction": "tx"})
        for w in self.data_conns:
            w.write(msg)
            await w.drain()

        # Simulate loopback for local tests
        for sub_topic, cb in self.data_subs:
            if topic == sub_topic or topic.startswith(sub_topic):
                cb(payload)

    async def subscribe(self, topic: str, callback: Callable[[bytes], None]):
        self.data_subs.append((topic, callback))

    async def step_clock(self, delta_ns: int):
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
        return resp.current_vtime_ns, resp.quantum_number


class FaultInjectingTransport(SimulationTransport):
    """
    DET-3: Wraps an existing SimulationTransport to introduce artificial
    packet loss and latency (Chaos Engineering) for deterministic robustness testing.
    """

    def __init__(self, inner: SimulationTransport, drop_prob: float = 0.0, delay_s: float = 0.0):
        self.inner = inner
        self.drop_prob = drop_prob
        self.delay_s = delay_s

    async def start(self):
        await self.inner.start()

    async def stop(self):
        await self.inner.stop()

    def get_clock_device_str(self, node_id: int) -> str:
        return self.inner.get_clock_device_str(node_id)

    def get_peripheral_props(self) -> str:
        return self.inner.get_peripheral_props()

    def dtb_router_endpoint(self) -> str:
        return self.inner.dtb_router_endpoint()

    async def publish(self, topic: str, payload: bytes):
        if self.drop_prob > 0.0 and random.random() < self.drop_prob:
            # Deliberately drop packet
            if hasattr(self.inner, "history"):
                self.inner.history.append(
                    {
                        "time": __import__("time").time(),
                        "topic": topic,
                        "payload": payload.hex(),
                        "direction": "tx_dropped",
                    }
                )
            return

        if self.delay_s > 0.0:
            await asyncio.sleep(self.delay_s)  # SLEEP_EXCEPTION: Chaos Engineering delay injection

        await self.inner.publish(topic, payload)

    async def subscribe(self, topic: str, callback):
        await self.inner.subscribe(topic, callback)

    async def step_clock(self, delta_ns: int):
        await self.inner.step_clock(delta_ns)

    def dump_flight_recorder(self) -> list[dict]:
        return getattr(self.inner, "dump_flight_recorder", lambda: [])()
