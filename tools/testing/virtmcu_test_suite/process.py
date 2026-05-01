"""
Context manager for background processes to guarantee strict cleanup.
Ensures processes are terminated, waited upon, and forcefully killed if necessary.
Also captures stdout and stderr in the background.
"""

from __future__ import annotations

import asyncio
import contextlib
import logging
from typing import TYPE_CHECKING, Any, cast

from tools.testing.utils import get_time_multiplier

if TYPE_CHECKING:
    from pathlib import Path
    from types import TracebackType


logger = logging.getLogger(__name__)


class AsyncManagedProcess:
    def __init__(
        self,
        *args: object,
        env: dict[str, str] | None = None,
        cwd: str | Path | None = None,
        graceful_timeout: float = 2.0,
        capture_output: bool = True,
        **kwargs: object,
    ) -> None:
        self.args = [str(a) for a in args]
        self.env = env
        self.cwd = cwd
        self.graceful_timeout = graceful_timeout
        self.capture_output = capture_output
        self.kwargs = kwargs
        self.proc: asyncio.subprocess.Process | None = None
        self.stdout_lines: list[str] = []
        self.stderr_lines: list[str] = []
        self._tasks: list[asyncio.Task[None]] = []
        self.output_event = asyncio.Event()

    async def wait_for_line(self, pattern: str, target: str = "stdout", timeout: float = 10.0) -> bool | None:
        if timeout is not None:
            timeout *= get_time_multiplier()
        import re

        regex = re.compile(pattern)
        loop = asyncio.get_running_loop()
        start = loop.time()

        while True:
            text = self.stdout_text if target == "stdout" else self.stderr_text
            if regex.search(text):
                return True

            elapsed = loop.time() - start
            if elapsed > timeout:
                return False

            try:
                await asyncio.wait_for(self.output_event.wait(), timeout=timeout - elapsed)
                self.output_event.clear()
            except TimeoutError:
                return False

    async def __aenter__(self) -> AsyncManagedProcess:
        self.proc = await asyncio.create_subprocess_exec(
            *self.args,
            env=self.env,
            cwd=self.cwd,
            stdout=asyncio.subprocess.PIPE if self.capture_output else None,
            stderr=asyncio.subprocess.PIPE if self.capture_output else None,
            **cast(Any, self.kwargs),
        )

        if self.capture_output:

            async def _stream(stream: asyncio.StreamReader, target_list: list[str]) -> None:
                while True:
                    line = await stream.readline()
                    if not line:
                        break
                    decoded = line.decode(errors="replace")
                    target_list.append(decoded)
                    self.output_event.set()

            if self.proc.stdout:
                self._tasks.append(asyncio.create_task(_stream(self.proc.stdout, self.stdout_lines)))
            if self.proc.stderr:
                self._tasks.append(asyncio.create_task(_stream(self.proc.stderr, self.stderr_lines)))

        return self

    async def wait(self, timeout: float | None = None) -> int:
        assert self.proc is not None
        if timeout:
            return await asyncio.wait_for(self.proc.wait(), timeout=timeout)
        return await self.proc.wait()

    @property
    def returncode(self) -> int | None:
        assert self.proc is not None
        return self.proc.returncode

    @property
    def stdout_text(self) -> str:
        return "".join(self.stdout_lines)

    @property
    def stderr_text(self) -> str:
        return "".join(self.stderr_lines)

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc_val: BaseException | None,
        exc_tb: TracebackType | None,
    ) -> None:
        if self.proc is None:
            return

        if self.proc.returncode is None:
            self.proc.terminate()
            try:
                await asyncio.wait_for(self.proc.wait(), timeout=self.graceful_timeout)
            except TimeoutError:
                logger.warning(f"Process {self.args[0]} did not terminate gracefully, killing it.")
                self.proc.kill()
                await self.proc.wait()

        # Clean up stream tasks
        for t in self._tasks:
            t.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await t
