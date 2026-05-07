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

from tools.testing.parameters import TestParams

logger = logging.getLogger(__name__)


def wait_for_zenoh_router(router_url: str, timeout: float = 15.0) -> bool:

    import zenoh

    from tools.testing.virtmcu_test_suite.conftest_core import make_client_config

    config = make_client_config(connect=router_url)

    start_time = time.time()
    while time.time() - start_time < timeout:
        try:
            temp_session = zenoh.open(  # virtmcu-allow: zenoh_open reasoning="config built by make_client_config"
                config
            )
            typing.cast(typing.Any, temp_session).close()
            return True
        except zenoh.ZError:
            time.sleep(0.1)  # virtmcu-allow: sleep reasoning="polling for router startup"
    logger.error(f"FAILED: Zenoh router failed to bind at {router_url} within {timeout} seconds")
    return False


def mock_execution_delay(seconds: float) -> None:
    """
    Test utility function to simulate execution delay in mock nodes,
    or to serve as a keepalive pause in standalone scripts.
    Replaces raw time sleep calls.
    """
    time.sleep(seconds)  # virtmcu-allow: sleep reasoning="mock_execution_delay"


async def yield_now() -> None:
    """
    SOTA Enterprise Grade yield: explicitly relinquishes control to the asyncio event loop.

    This ensures that background tasks (like Zenoh subscribers, QMP readers, or
    process stream pipes) have a chance to run. Equivalent to asyncio sleep zero
    but centralized for architectural consistency and to avoid repeating
    SLEEP_EXCEPTION markers for deliberate yielding.
    """
    await asyncio.sleep(0)  # virtmcu-allow: sleep reasoning="explicit yield to event loop"


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
        await asyncio.wait_for(event.wait(), timeout=TestParams.scale_timeout(timeout))
    finally:
        observer.stop()
        observer.join()
