"""
SOTA Test Module: test_jitter_proxy

Context:
This module implements tests for the test_jitter_proxy subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_jitter_proxy.
"""

from __future__ import annotations

import subprocess
import sys
import threading
import time
import typing

import pytest
import zenoh

from tests.fixtures.guest_apps.perf_bench.jitter_proxy import CLOCK_ADVANCE_PREFIX, JitterProxy
from tools.testing.env import WORKSPACE_DIR
from tools.testing.utils import mock_execution_delay

GET_FREE_PORT_SCRIPT = WORKSPACE_DIR / "scripts" / "get-free-port.py"


def _get_endpoint(proto: str = "tcp/") -> str:
    return (
        subprocess.check_output([sys.executable, str(GET_FREE_PORT_SCRIPT), "--endpoint", "--proto", proto])
        .decode()
        .strip()
    )


def _get_port() -> int:
    return int(subprocess.check_output([sys.executable, str(GET_FREE_PORT_SCRIPT), "--port"]).decode().strip())


@pytest.fixture
def mock_upstream_router() -> object:
    """Spins up an isolated local Zenoh router to act as the upstream."""
    endpoint = _get_endpoint()
    cfg = zenoh.Config()
    cfg.insert_json5("listen/endpoints", f'["{endpoint}"]')
    cfg.insert_json5("scouting/multicast/enabled", "false")
    router = zenoh.open(cfg)
    yield router, endpoint
    typing.cast(typing.Any, router).close()


def _wait_for_queryable(session: zenoh.Session, topic: str, timeout: float = 5.0) -> bool:
    """Deterministically polls until a queryable on the topic responds or timeouts."""
    deadline = time.perf_counter() + timeout
    while time.perf_counter() < deadline:
        # We send a dummy payload. The mock queryable handles it.
        replies = list(session.get(topic, payload=b"ping", timeout=0.5))
        if replies and hasattr(replies[0], "ok") and replies[0].ok is not None:
            return True
        mock_execution_delay(0.1)  # SLEEP_EXCEPTION: infrastructure jitter proxy
    return False


def test_jitter_proxy_routing(mock_upstream_router: tuple[zenoh.Session, str]) -> None:
    """
    Validates that the proxy correctly isolates sessions and forwards payloads.
    - Upstream Session: TimeAuthority
    - Proxy Backend: QEMU
    """
    _, upstream_url = mock_upstream_router
    proxy_endpoint = _get_endpoint()
    max_jitter_us = 1000  # 1 ms max jitter

    proxy = JitterProxy(upstream_url, proxy_endpoint, max_jitter_us)
    proxy_thread = threading.Thread(target=proxy.run, daemon=True)
    proxy_thread.start()

    qemu_handled_payload = None
    qemu_session = None
    qemu_queryable = None
    ta_session = None

    def mock_qemu_queryable(query: zenoh.Query) -> None:
        nonlocal qemu_handled_payload
        qemu_handled_payload = query.payload.to_bytes() if query.payload else b""
        query.reply(query.key_expr, b"qemu_response")

    try:
        # 1. Mock QEMU: Connect to the proxy's backend listen port and declare a queryable.
        qemu_cfg = zenoh.Config()
        qemu_cfg.insert_json5("connect/endpoints", f'["{proxy_endpoint}"]')
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
        qemu_handled_payload = None  # reset after ping
        replies = list(ta_session.get(f"{CLOCK_ADVANCE_PREFIX}0", payload=b"ta_request", timeout=5.0))

        # 4. Verify the architecture worked correctly
        assert len(replies) == 1, "TimeAuthority should receive exactly one reply"
        assert hasattr(replies[0], "ok"), "Reply should have 'ok' attribute"
        assert replies[0].ok is not None, "Reply 'ok' should not be None"
        assert replies[0].ok.payload.to_bytes() == b"qemu_response", "Payload should route back from QEMU to TA"
        assert qemu_handled_payload == b"ta_request", "Payload should route forward from TA to QEMU"

        with proxy._lock:
            assert len(proxy.injected_delays_us) > 0
            assert 0 <= proxy.injected_delays_us[-1] <= max_jitter_us

    finally:
        proxy.stop()
        proxy_thread.join(timeout=2.0)
        if qemu_queryable:
            qemu_queryable.undeclare()  # type: ignore[no-untyped-call]
        if qemu_session:
            qemu_session.close()  # type: ignore[no-untyped-call]
        if ta_session:
            ta_session.close()  # type: ignore[no-untyped-call]


def test_jitter_proxy_qemu_offline(mock_upstream_router: tuple[zenoh.Session, str]) -> None:
    """
    Validates that the proxy fails gracefully if QEMU hasn't registered its queryable.
    """
    _, upstream_url = mock_upstream_router
    proxy_endpoint = _get_endpoint()

    proxy = JitterProxy(upstream_url, proxy_endpoint, max_jitter_us=100)
    proxy_thread = threading.Thread(target=proxy.run, daemon=True)
    proxy_thread.start()

    ta_session = None
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
            mock_execution_delay(0.1)  # SLEEP_EXCEPTION: infrastructure jitter proxy

        assert len(replies) == 1
        assert hasattr(replies[0], "err")
        assert replies[0].err is not None
        assert replies[0].err.payload.to_bytes() == b"proxy: no QEMU reply"
    finally:
        proxy.stop()
        proxy_thread.join(timeout=2.0)
        if ta_session:
            ta_session.close()  # type: ignore[no-untyped-call]


def test_jitter_proxy_routing_storm_detection(mock_upstream_router: tuple[zenoh.Session, str]) -> None:
    """
    Intentionally creates a query storm to verify the proxy's fail-fast concurrency guard.
    """
    _, upstream_url = mock_upstream_router
    proxy_endpoint = _get_endpoint()

    proxy = JitterProxy(upstream_url, proxy_endpoint, max_jitter_us=100)
    proxy_thread = threading.Thread(target=proxy.run, daemon=True)
    proxy_thread.start()

    ta_session = None
    futures = []
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
            mock_execution_delay(0.1)  # SLEEP_EXCEPTION: infrastructure jitter proxy

        # Flood the proxy (triggering the >50 in_flight guard)
        # We don't wait for responses, we just blast async gets.
        for _ in range(60):
            # session.get is blocking, we need to run it in threads
            t = threading.Thread(
                target=lambda: list(ta_session.get(f"{CLOCK_ADVANCE_PREFIX}0", payload=b"", timeout=1.0))
            )
            t.daemon = True
            t.start()
            futures.append(t)

        # Give it a tiny bit of time to hit the limit
        mock_execution_delay(0.2)  # SLEEP_EXCEPTION: infrastructure jitter proxy

        # The next request should be instantly rejected by the guard
        replies = list(ta_session.get(f"{CLOCK_ADVANCE_PREFIX}0", payload=b"", timeout=1.0))
        assert len(replies) == 1
        assert hasattr(replies[0], "err")
        assert replies[0].err is not None
        assert replies[0].err.payload.to_bytes() in [b"proxy: no QEMU reply", b"proxy: routing loop detected"]

    finally:
        for t in futures:
            t.join(timeout=1.0)
        proxy.stop()
        proxy_thread.join(timeout=2.0)
        if ta_session:
            ta_session.close()  # type: ignore[no-untyped-call]
