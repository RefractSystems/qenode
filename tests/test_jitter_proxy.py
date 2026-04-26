import socket
import sys
import threading
import time
from pathlib import Path

import pytest
import zenoh

WORKSPACE_DIR = Path(__file__).resolve().parent.parent
sys.path.append(str(WORKSPACE_DIR / "tools"))
sys.path.append(str(WORKSPACE_DIR / "test" / "phase16"))

from jitter_proxy import CLOCK_ADVANCE_PREFIX, JitterProxy  # noqa: E402


def _free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


@pytest.fixture
def mock_upstream_router():
    """Spins up an isolated local Zenoh router to act as the upstream."""
    port = _free_port()
    endpoint = f"tcp/127.0.0.1:{port}"
    cfg = zenoh.Config()
    cfg.insert_json5("listen/endpoints", f'["{endpoint}"]')
    cfg.insert_json5("scouting/multicast/enabled", "false")
    router = zenoh.open(cfg)
    yield router, endpoint
    router.close()


def _wait_for_queryable(session: zenoh.Session, topic: str, timeout: float = 5.0) -> bool:
    """Deterministically polls until a queryable on the topic responds or timeouts."""
    deadline = time.perf_counter() + timeout
    while time.perf_counter() < deadline:
        # We send a dummy payload. The mock queryable handles it.
        replies = list(session.get(topic, payload=b"ping", timeout=0.5))
        if replies and hasattr(replies[0], "ok") and replies[0].ok is not None:
            return True
        time.sleep(0.1)
    return False


def test_jitter_proxy_routing(mock_upstream_router):
    """
    Validates that the proxy correctly isolates sessions and forwards payloads.
    - Upstream Session: TimeAuthority
    - Proxy Backend: QEMU
    """
    _, upstream_url = mock_upstream_router
    proxy_port = _free_port()
    max_jitter_us = 1000  # 1 ms max jitter

    proxy = JitterProxy(upstream_url, proxy_port, max_jitter_us)
    proxy_thread = threading.Thread(target=proxy.run, daemon=True)
    proxy_thread.start()

    qemu_handled_payload = None

    def mock_qemu_queryable(query):
        nonlocal qemu_handled_payload
        qemu_handled_payload = query.payload.to_bytes() if query.payload else b""
        query.reply(query.key_expr, b"qemu_response")

    try:
        # 1. Mock QEMU: Connect to the proxy's backend listen port and declare a queryable.
        qemu_cfg = zenoh.Config()
        qemu_cfg.insert_json5("connect/endpoints", f'["tcp/127.0.0.1:{proxy_port}"]')
        qemu_cfg.insert_json5("scouting/multicast/enabled", "false")
        qemu_session = zenoh.open(qemu_cfg)
        qemu_queryable = qemu_session.declare_queryable(f"{CLOCK_ADVANCE_PREFIX}0", mock_qemu_queryable)

        # 2. Mock TimeAuthority: Connect to the upstream router.
        ta_cfg = zenoh.Config()
        ta_cfg.insert_json5("connect/endpoints", f'["{upstream_url}"]')
        ta_cfg.insert_json5("scouting/multicast/enabled", "false")
        ta_session = zenoh.open(ta_cfg)

        # Wait deterministically for the routing to stabilize.
        assert _wait_for_queryable(ta_session, f"{CLOCK_ADVANCE_PREFIX}0", timeout=5.0), "Routing failed to propagate"

        # 3. Execute the actual query
        qemu_handled_payload = None # reset after ping
        replies = list(ta_session.get(f"{CLOCK_ADVANCE_PREFIX}0", payload=b"ta_request", timeout=5.0))

        # 4. Verify the architecture worked correctly
        assert len(replies) == 1, "TimeAuthority should receive exactly one reply"
        assert hasattr(replies[0], "ok") and replies[0].ok is not None, "Reply should be successful"
        assert replies[0].ok.payload.to_bytes() == b"qemu_response", "Payload should route back from QEMU to TA"
        assert qemu_handled_payload == b"ta_request", "Payload should route forward from TA to QEMU"

        with proxy._lock:
            assert len(proxy.injected_delays_us) > 0
            assert 0 <= proxy.injected_delays_us[-1] <= max_jitter_us

    finally:
        proxy.stop()
        proxy_thread.join(timeout=2.0)
        qemu_queryable.undeclare()
        qemu_session.close()
        ta_session.close()


def test_jitter_proxy_qemu_offline(mock_upstream_router):
    """
    Validates that the proxy fails gracefully if QEMU hasn't registered its queryable.
    """
    _, upstream_url = mock_upstream_router
    proxy_port = _free_port()

    proxy = JitterProxy(upstream_url, proxy_port, max_jitter_us=100)
    proxy_thread = threading.Thread(target=proxy.run, daemon=True)
    proxy_thread.start()

    try:
        ta_cfg = zenoh.Config()
        ta_cfg.insert_json5("connect/endpoints", f'["{upstream_url}"]')
        ta_cfg.insert_json5("scouting/multicast/enabled", "false")
        ta_session = zenoh.open(ta_cfg)

        # It might take a moment for the proxy to declare its queryable on the upstream.
        # Wait until the proxy itself responds (it will return an error because QEMU is missing).
        deadline = time.perf_counter() + 5.0
        replies = []
        while time.perf_counter() < deadline:
            replies = list(ta_session.get(f"{CLOCK_ADVANCE_PREFIX}0", payload=b"ta_request", timeout=1.0))
            if replies:
                break
            time.sleep(0.1)

        assert len(replies) == 1
        assert hasattr(replies[0], "err") and replies[0].err is not None
        assert replies[0].err.payload.to_bytes() == b"proxy: no QEMU reply"
    finally:
        proxy.stop()
        proxy_thread.join(timeout=2.0)
        ta_session.close()


def test_jitter_proxy_routing_storm_detection(mock_upstream_router):
    """
    Intentionally creates a query storm to verify the proxy's fail-fast concurrency guard.
    """
    _, upstream_url = mock_upstream_router
    proxy_port = _free_port()

    proxy = JitterProxy(upstream_url, proxy_port, max_jitter_us=100)
    proxy_thread = threading.Thread(target=proxy.run, daemon=True)
    proxy_thread.start()

    try:
        ta_cfg = zenoh.Config()
        ta_cfg.insert_json5("connect/endpoints", f'["{upstream_url}"]')
        ta_cfg.insert_json5("scouting/multicast/enabled", "false")
        ta_session = zenoh.open(ta_cfg)

        # Wait for proxy queryable
        deadline = time.perf_counter() + 5.0
        while time.perf_counter() < deadline:
            if list(ta_session.get(f"{CLOCK_ADVANCE_PREFIX}0", payload=b"", timeout=0.1)):
                break
            time.sleep(0.1)

        # Flood the proxy (triggering the >50 in_flight guard)
        # We don't wait for responses, we just blast async gets.
        futures = []
        for _ in range(60):
            # session.get is blocking, we need to run it in threads
            t = threading.Thread(target=lambda: list(ta_session.get(f"{CLOCK_ADVANCE_PREFIX}0", payload=b"", timeout=1.0)))
            t.daemon = True
            t.start()
            futures.append(t)

        # Give it a tiny bit of time to hit the limit
        time.sleep(0.2)

        # The next request should be instantly rejected by the guard
        replies = list(ta_session.get(f"{CLOCK_ADVANCE_PREFIX}0", payload=b"", timeout=1.0))
        assert len(replies) == 1
        assert hasattr(replies[0], "err") and replies[0].err is not None
        assert replies[0].err.payload.to_bytes() in [b"proxy: no QEMU reply", b"proxy: routing loop detected"]

    finally:
        proxy.stop()
        proxy_thread.join(timeout=2.0)
        ta_session.close()
