import asyncio
import os
import shutil
import signal
import subprocess
import tempfile
import time
from typing import List, Optional, Tuple, Union

from tools.testing.qmp_bridge import QmpBridge


class QemuLibrary:
    """
    Robot Framework library for controlling QEMU via QMP.
    Provides a synchronous interface to the asynchronous QmpBridge.
    """

    ROBOT_LIBRARY_SCOPE = "GLOBAL"

    def __init__(self):
        self.bridge = QmpBridge()
        # Robot Framework is synchronous; create a dedicated event loop for the session.
        # Never use get_event_loop() here — it is deprecated in Python 3.10+ when no
        # running loop exists, and raises RuntimeError in 3.12.
        self.loop = asyncio.new_event_loop()
        asyncio.set_event_loop(self.loop)
        self.proc: Optional[subprocess.Popen] = None
        self.tmpdir: Optional[str] = None

    def _run(self, coro):
        return self.loop.run_until_complete(coro)

    def launch_qemu(self, dtb_path: str, kernel_path: Optional[str] = None, extra_args: Optional[Union[str, List[str]]] = None) -> Tuple[str, str]:
        """
        Launches QEMU using the run.sh script and returns the QMP and UART socket paths.
        """
        tmpdir = tempfile.mkdtemp(prefix="virtmcu-robot-")
        qmp_sock = os.path.join(tmpdir, "qmp.sock")
        uart_sock = os.path.join(tmpdir, "uart.sock")

        workspace_root = os.getcwd()
        run_script = os.path.join(workspace_root, "scripts/run.sh")

        cmd = [run_script, "--dtb", os.path.abspath(dtb_path)]
        if kernel_path:
            cmd.extend(["--kernel", os.path.abspath(kernel_path)])

        cmd.extend(
            [
                "-qmp",
                f"unix:{qmp_sock},server,nowait",
                "-serial",
                f"unix:{uart_sock},server,nowait",
                "-display",
                "none",
                "-nographic",
            ]
        )

        if extra_args:
            if isinstance(extra_args, str):
                cmd.extend(extra_args.split())
            else:
                cmd.extend(extra_args)

        self.proc = subprocess.Popen(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=os.environ.copy(),
            start_new_session=True,  # Replace preexec_fn=os.setsid
        )
        self.tmpdir = tmpdir

        # Wait for sockets
        for _ in range(100):
            if self.proc.poll() is not None:
                stdout, stderr = self.proc.communicate()
                raise RuntimeError(
                    f"QEMU exited unexpectedly (rc={self.proc.returncode}) before sockets appeared.\n"
                    f"STDOUT: {stdout.decode()}\nSTDERR: {stderr.decode()}"
                )
            if os.path.exists(qmp_sock) and os.path.exists(uart_sock):
                break
            time.sleep(0.1)
        else:
            self.proc.terminate()
            stdout, stderr = self.proc.communicate()
            raise RuntimeError(
                f"QEMU sockets did not appear in time. STDOUT: {stdout.decode()} STDERR: {stderr.decode()}"
            )

        return qmp_sock, uart_sock

    def connect_to_qemu(self, qmp_socket_path: str, uart_socket_path: Optional[str] = None):
        """
        Connects to the QEMU QMP and UART sockets.
        """
        self._run(self.bridge.connect(qmp_socket_path, uart_socket_path))

    def start_emulation(self):
        """
        Starts or resumes the emulation.
        """
        self._run(self.bridge.start_emulation())

    def pause_emulation(self):
        """
        Pauses the emulation.
        """
        self._run(self.bridge.pause_emulation())

    def reset_emulation(self):
        """
        Resets the emulation.
        """
        self._run(self.bridge.execute("system_reset"))

    def wait_for_line_on_uart(self, pattern: str, timeout: Union[float, str] = 10.0):
        """
        Waits for a specific pattern to appear on the UART.
        """
        found = self._run(self.bridge.wait_for_line_on_uart(pattern, float(timeout)))
        if not found:
            raise AssertionError(
                f"Pattern '{pattern}' not found on UART within {timeout}s. "
                f"Current buffer: {repr(self.bridge.uart_buffer)}"
            )

    def write_to_uart(self, text: str):
        """
        Writes text to the UART socket.
        """
        self._run(self.bridge.write_to_uart(text))

    def pc_should_be_equal(self, expected_pc: Union[int, str]):
        """
        Asserts that the current Program Counter is equal to the expected value.
        """
        actual_pc = self._run(self.bridge.get_pc())
        expected = int(expected_pc, 0) if isinstance(expected_pc, str) else expected_pc
        if actual_pc != expected:
            raise AssertionError(f"PC expected to be {hex(expected)}, but was {hex(actual_pc)}")

    def execute_monitor_command(self, command: str) -> str:
        """
        Executes a Human Monitor Command (HMP) and returns the output.
        """
        return self._run(self.bridge.execute("human-monitor-command", {"command-line": command}))

    def close_all_connections(self):
        """
        Closes all QMP and UART connections and cleans up the QEMU process.
        """
        self._run(self.bridge.close())
        if self.proc:
            try:
                os.killpg(os.getpgid(self.proc.pid), signal.SIGTERM)
                self.proc.wait(timeout=5)
            except Exception:
                if self.proc:
                    self.proc.kill()
            self.proc = None

        if self.tmpdir:
            shutil.rmtree(self.tmpdir, ignore_errors=True)
            self.tmpdir = None

        if self.loop.is_running():
             # Should not be running if _run finished
             pass
        self.loop.close()

