import asyncio
import contextlib
import logging

logger = logging.getLogger(__name__)

class AsyncManagedProcess:
    """
    Context manager for background processes to guarantee strict cleanup.
    Ensures processes are terminated, waited upon, and forcefully killed if necessary.
    Also captures stdout and stderr in the background.
    """

    def __init__(self, *args, env=None, cwd=None, graceful_timeout: float = 2.0, capture_output: bool = True, **kwargs):
        self.args = [str(a) for a in args]
        self.env = env
        self.cwd = cwd
        self.graceful_timeout = graceful_timeout
        self.capture_output = capture_output
        self.kwargs = kwargs
        self.proc: asyncio.subprocess.Process | None = None
        self.stdout_lines: list[str] = []
        self.stderr_lines: list[str] = []
        self._tasks: list[asyncio.Task] = []
        self.output_event = asyncio.Event()

    async def wait_for_line(self, pattern: str, target: str = "stdout", timeout: float = 10.0) -> bool:
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

    async def __aenter__(self):
        self.proc = await asyncio.create_subprocess_exec(
            *self.args,
            env=self.env,
            cwd=self.cwd,
            stdout=asyncio.subprocess.PIPE if self.capture_output else None,
            stderr=asyncio.subprocess.PIPE if self.capture_output else None,
            **self.kwargs
        )

        if self.capture_output:
            async def _stream(stream, target_list):
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

    async def wait(self, timeout=None):
        assert self.proc is not None
        if timeout:
            return await asyncio.wait_for(self.proc.wait(), timeout=timeout)
        return await self.proc.wait()

    @property
    def returncode(self):
        assert self.proc is not None
        return self.proc.returncode

    @property
    def stdout_text(self):
        return "".join(self.stdout_lines)

    @property
    def stderr_text(self):
        return "".join(self.stderr_lines)

    async def __aexit__(self, exc_type, exc_val, exc_tb):
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
