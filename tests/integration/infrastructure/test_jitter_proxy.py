# ZENOH_HACK_EXCEPTION: Uses peer-mode session and queryables for jitter proxy testing
"""
SOTA Test Module: test_jitter_proxy

Context:
This module implements tests for the test_jitter_proxy subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_jitter_proxy.
"""

from __future__ import annotations

import asyncio
import sys
import time
import typing
from collections.abc import Generator

import pytest
import zenoh

from tools.testing.env import WORKSPACE_DIR
from tools.testing.utils import get_time_multiplier
from tools.testing.virtmcu_test_suite.conftest_core import (
    ManagedSubprocess,
    get_free_port,
    open_client_session,
    wait_for_zenoh_discovery,
)
from tools.testing.virtmcu_test_suite.topics import SimTopic


@pytest.fixture
def mock_upstream_router() -> Generator[tuple[zenoh.Session, str]]:
    """Spins up an isolated local Zenoh router to act as the upstream."""
    endpoint = get_free_port()
    cfg = zenoh.Config()
    cfg.insert_json5("listen/endpoints", f'["{endpoint}"]')
    cfg.insert_json5("scouting/multicast/enabled", "false")
    router = zenoh.open(cfg)  # ZENOH_OPEN_EXCEPTION: peer/router-mode session for an isolated test router
    yield router, endpoint
    router.close()  # type: ignore[no-untyped-call]


async def _wait_for_queryable_async(session: zenoh.Session, topic: str, timeout: float = 5.0) -> bool:
    """Deterministically polls until a queryable on the topic responds or timeouts."""
    deadline = time.perf_counter() + timeout
    while time.perf_counter() < deadline:
        try:
            replies = list(session.get(topic, payload=b"ping", timeout=0.5))
            if replies and hasattr(replies[0], "ok") and replies[0].ok is not None:
                return True
        except zenoh.ZError:
            pass
        await asyncio.sleep(0.1)  # SLEEP_EXCEPTION: Intentional delay for proxy test
    return False


@pytest.mark.asyncio
async def test_jitter_proxy_routing(mock_upstream_router: tuple[zenoh.Session, str]) -> None:
    """
    Validates that the proxy correctly isolates sessions and forwards payloads.
    """
    _, upstream_url = mock_upstream_router
    proxy_endpoint = get_free_port()
    max_jitter_us = 1000

    proxy_script = WORKSPACE_DIR / "tests/fixtures/guest_apps/perf_bench/jitter_proxy.py"
    cmd = [sys.executable, "-u", str(proxy_script), upstream_url, proxy_endpoint, str(max_jitter_us)]

    qemu_handled_payload = None
    qemu_session = None
    qemu_queryable = None
    ta_session = None

    def mock_qemu_queryable(query: zenoh.Query) -> None:
        nonlocal qemu_handled_payload
        qemu_handled_payload = query.payload.to_bytes() if query.payload else b""
        query.reply(query.key_expr, b"qemu_response")

    async with ManagedSubprocess("jitter_proxy", cmd) as proc:
        try:
            assert await proc.wait_for_line("listen=tcp/")
            await asyncio.sleep(0.5 * get_time_multiplier())  # SLEEP_EXCEPTION: Intentional delay for proxy test

            ta_session = open_client_session(connect=upstream_url)
            await wait_for_zenoh_discovery(ta_session, SimTopic.plugin_liveliness("jitter_proxy", 0))

            qemu_session = open_client_session(connect=proxy_endpoint)
            qemu_queryable = qemu_session.declare_queryable(SimTopic.clock_advance(0), mock_qemu_queryable)

            assert await _wait_for_queryable_async(ta_session, SimTopic.clock_advance(0), timeout=5.0)

            qemu_handled_payload = None
            replies = list(ta_session.get(SimTopic.clock_advance(0), payload=b"ta_request", timeout=5.0))

            assert len(replies) == 1
            assert replies[0].ok is not None
            assert replies[0].ok.payload.to_bytes() == b"qemu_response"
            assert qemu_handled_payload == b"ta_request"

            await proc.stop()
            assert await proc.wait_for_line("injected.*delays")

        finally:
            if qemu_queryable:
                typing.cast(typing.Any, qemu_queryable).undeclare()
            if qemu_session:
                typing.cast(typing.Any, qemu_session).close()
            if ta_session:
                typing.cast(typing.Any, ta_session).close()


@pytest.mark.asyncio
async def test_jitter_proxy_qemu_offline(mock_upstream_router: tuple[zenoh.Session, str]) -> None:
    """Validates proxy fail-gracefully behavior."""
    _, upstream_url = mock_upstream_router
    proxy_endpoint = get_free_port()

    proxy_script = WORKSPACE_DIR / "tests/fixtures/guest_apps/perf_bench/jitter_proxy.py"
    cmd = [sys.executable, "-u", str(proxy_script), upstream_url, proxy_endpoint, "100"]

    async with ManagedSubprocess("jitter_proxy", cmd) as proc:
        assert await proc.wait_for_line("listen=tcp/")
        await asyncio.sleep(0.5 * get_time_multiplier())  # SLEEP_EXCEPTION: Intentional delay for proxy test

        ta_session = open_client_session(connect=upstream_url)
        try:
            await wait_for_zenoh_discovery(ta_session, SimTopic.plugin_liveliness("jitter_proxy", 0))

            deadline = time.perf_counter() + 5.0
            replies = []
            while time.perf_counter() < deadline:
                replies = list(ta_session.get(SimTopic.clock_advance(0), payload=b"ta_request", timeout=1.0))
                if replies:
                    break
                await asyncio.sleep(0.1)  # SLEEP_EXCEPTION: Intentional delay for proxy test

            assert len(replies) == 1
            assert hasattr(replies[0], "err")
            assert replies[0].err is not None
            assert replies[0].err.payload.to_bytes() == b"proxy: no QEMU reply"
        finally:
            if ta_session:
                typing.cast(typing.Any, ta_session).close()


@pytest.mark.skip(reason="Zenoh Python API handles queries sequentially, preventing in_flight from exceeding limit")
@pytest.mark.asyncio
async def test_jitter_proxy_routing_storm_detection(mock_upstream_router: tuple[zenoh.Session, str]) -> None:
    """Intentionally creates a query storm."""
    _, upstream_url = mock_upstream_router
    proxy_endpoint = get_free_port()

    proxy_script = WORKSPACE_DIR / "tests/fixtures/guest_apps/perf_bench/jitter_proxy.py"
    # Run proxy with limit=1 to absolutely guarantee the query storm is detected
    cmd = [sys.executable, "-u", str(proxy_script), upstream_url, proxy_endpoint, "100", "1"]
    from concurrent.futures import ThreadPoolExecutor

    with ThreadPoolExecutor(max_workers=50) as executor:
        async with ManagedSubprocess("jitter_proxy", cmd) as proc:
            assert await proc.wait_for_line("listen=tcp/")
            await asyncio.sleep(0.5 * get_time_multiplier())  # SLEEP_EXCEPTION: Intentional delay for proxy test

            ta_session = open_client_session(connect=upstream_url)
            backend_session = open_client_session(connect=proxy_endpoint)

            def slow_queryable(query: zenoh.Query) -> None:
                import threading

                def delayed_reply() -> None:
                    time.sleep(2.0)  # SLEEP_EXCEPTION: Intentional delay for proxy test
                    query.reply(query.key_expr, b"slow_reply")

                threading.Thread(target=delayed_reply, daemon=True).start()

            sub = backend_session.declare_queryable(SimTopic.clock_advance(0), slow_queryable)

            try:
                await wait_for_zenoh_discovery(ta_session, SimTopic.plugin_liveliness("jitter_proxy", 0))

                loop = asyncio.get_running_loop()
                sessions = [open_client_session(connect=upstream_url) for _ in range(5)]

                async def fire_get(sess: zenoh.Session) -> list[zenoh.Reply]:

                    return await loop.run_in_executor(
                        executor, lambda: list(sess.get(SimTopic.clock_advance(0), timeout=10.0))
                    )

                tasks = []
                for s in sessions:
                    for _ in range(10):
                        tasks.append(asyncio.create_task(fire_get(s)))

                await asyncio.wait(tasks, timeout=5.0)
                assert await proc.wait_for_line("query storm detected")

                replies = list(ta_session.get(SimTopic.clock_advance(0), timeout=1.0))
                assert len(replies) == 1
                assert hasattr(replies[0], "err")
                assert replies[0].err is not None
                assert replies[0].err.payload.to_bytes() in [b"proxy: no QEMU reply", b"proxy: routing loop detected"]
            finally:
                for s in sessions:
                    typing.cast(typing.Any, s).close()
                typing.cast(typing.Any, sub).undeclare()
                typing.cast(typing.Any, backend_session).close()
                typing.cast(typing.Any, ta_session).close()
