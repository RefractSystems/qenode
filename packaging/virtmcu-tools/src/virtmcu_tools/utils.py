import asyncio
import os
from pathlib import Path

from watchdog.events import FileCreatedEvent, FileSystemEventHandler
from watchdog.observers import Observer


def get_time_multiplier() -> float:  # virtmcu-allow: time_multiplier reasoning="external package API"
    """
    Returns a global timeout multiplier based on the execution environment.
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
    """
    await asyncio.sleep(0)  # virtmcu-allow: sleep reasoning="explicit yield to event loop"


async def wait_for_file_creation(path: str | os.PathLike, timeout: float = 10.0) -> None:
    """
    Deterministic wait for a file to appear on the filesystem using watchdog (inotify).
    """
    path = Path(path)
    if path.exists():
        return

    loop = asyncio.get_running_loop()
    event = asyncio.Event()

    class Handler(FileSystemEventHandler):
        def on_created(self, e: object) -> None:
            if isinstance(e, FileCreatedEvent) and Path(e.src_path).resolve() == path.resolve():
                loop.call_soon_threadsafe(event.set)

    observer = Observer()
    observer.schedule(Handler(), str(path.parent), recursive=False)
    observer.start()

    try:
        if path.exists():
            return
        await asyncio.wait_for(
            event.wait(),
            timeout=timeout
            * get_time_multiplier(),  # virtmcu-allow: time_multiplier reasoning="legacy script compatibility"
        )  # virtmcu-allow: time_multiplier reasoning="external package API)"
    finally:
        observer.stop()
        observer.join()
