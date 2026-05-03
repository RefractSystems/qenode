"""
Deterministically wait for a Zenoh router to become available.
Returns True if successful, False if it timed out.
"""

import asyncio
import logging
import os
import time
import typing
from pathlib import Path

logger = logging.getLogger(__name__)


def wait_for_zenoh_router(router_url: str, timeout: float = 15.0) -> bool:

    import zenoh

    from tools.testing.virtmcu_test_suite.conftest_core import make_client_config

    config = make_client_config(connect=router_url)

    start_time = time.time()
    while time.time() - start_time < timeout:
        try:
            temp_session = zenoh.open(  # ZENOH_OPEN_EXCEPTION: config built by make_client_config
                config
            )
            typing.cast(typing.Any, temp_session).close()
            return True
        except zenoh.ZError:
            time.sleep(0.1)  # SLEEP_EXCEPTION: polling for router startup

    logger.error(f"FAILED: Zenoh router failed to bind at {router_url} within {timeout} seconds")
    return False


def mock_execution_delay(seconds: float) -> None:
    """
    Test utility function to simulate execution delay in mock nodes,
    or to serve as a keepalive pause in standalone scripts.
    Replaces raw time.sleep() calls.  # SLEEP_EXCEPTION:
    """
    time.sleep(seconds)  # SLEEP_EXCEPTION:


def get_time_multiplier() -> float:
    """
    Returns a global timeout multiplier based on the execution environment.
    Users and CI define VIRTMCU_ENV_PROFILE, the framework handles the math.
    """
    if os.environ.get("VIRTMCU_USE_ASAN") == "1":
        return 5.0  # ASan is ~5x slower
    if os.environ.get("VIRTMCU_USE_TSAN") == "1":
        return 10.0  # TSan is ~10x slower
    if os.environ.get("CI") == "true":
        return 2.0  # Standard CI buffer
    return 1.0  # Local developer machine


async def yield_now() -> None:
    """
    SOTA Enterprise Grade yield: explicitly relinquishes control to the asyncio event loop.

    This ensures that background tasks (like Zenoh subscribers, QMP readers, or
    process stream pipes) have a chance to run. Equivalent to asyncio sleep zero
    but centralized for architectural consistency and to avoid repeating
    SLEEP_EXCEPTION markers for deliberate yielding.
    """
    await asyncio.sleep(0)  # SLEEP_EXCEPTION: explicit yield to event loop


async def wait_for_file_creation(path: str | Path, timeout: float = 10.0) -> None:
    """
    Deterministic wait for a file to appear on the filesystem using watchdog (inotify).
    """
    path = Path(path)
    if path.exists():
        return

    from watchdog.events import FileCreatedEvent, FileSystemEventHandler
    from watchdog.observers import Observer

    loop = asyncio.get_running_loop()
    event = asyncio.Event()

    class Handler(FileSystemEventHandler):
        def on_created(self, e: object) -> None:
            if isinstance(e, FileCreatedEvent) and Path(os.fsdecode(e.src_path)).resolve() == Path(path).resolve():
                loop.call_soon_threadsafe(event.set)

    observer = Observer()
    observer.schedule(Handler(), str(Path(path).parent), recursive=False)
    observer.start()

    try:
        if path.exists():
            return
        await asyncio.wait_for(event.wait(), timeout=timeout * get_time_multiplier())
    finally:
        observer.stop()
        observer.join()
