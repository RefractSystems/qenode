from __future__ import annotations

import asyncio
import io
import logging
import os
import shutil
import sys
import tempfile
from contextlib import redirect_stderr, redirect_stdout, suppress
from pathlib import Path
from typing import TYPE_CHECKING

from virtmcu_tools.qmp_bridge import QmpBridge

if TYPE_CHECKING:
    import zenoh

logger = logging.getLogger(__name__)


class NodeContext:
    """Context for a single MCU node."""

    def __init__(self, node_id: str) -> None:
        self.node_id = node_id
        self.process: asyncio.subprocess.Process | None = None
        self.qmp_bridge: QmpBridge = QmpBridge()
        self.tmpdir: str = tempfile.mkdtemp(prefix=f"virtmcu-mcp-{node_id}-")
        self.qmp_socket_path: str = str(Path(self.tmpdir) / "qmp.sock")
        self.uart_socket_path: str = str(Path(self.tmpdir) / "uart.sock")
        self.yaml_path: str | None = None
        self.firmware_path: str | Path | None = None

    def cleanup(self) -> None:
        """Clean up temporary directory and sockets."""
        if Path(self.tmpdir).exists():
            shutil.rmtree(self.tmpdir, ignore_errors=True)


class NodeManager:
    """Manages multiple MCU nodes."""

    def __init__(self) -> None:
        self.nodes: dict[str, NodeContext] = {}
        self._zenoh_session: zenoh.Session | None = None

    def get_zenoh_session(self) -> zenoh.Session:
        """Returns the Zenoh session, opening it if necessary."""
        import zenoh

        if self._zenoh_session is None:
            self._zenoh_session = zenoh.open(zenoh.Config())
        return self._zenoh_session

    async def close(self) -> None:
        """Close all nodes and the Zenoh session."""
        for node in list(self.nodes.values()):
            await self.stop_node(node.node_id)
        if self._zenoh_session:
            self._zenoh_session.close()
            self._zenoh_session = None

    def get_node(self, node_id: str) -> NodeContext:
        """Retrieve or create a NodeContext for the given ID."""
        if node_id not in self.nodes:
            self.nodes[node_id] = NodeContext(node_id)
        return self.nodes[node_id]

    async def provision_board(self, node_id: str, board_config: str, config_type: str = "yaml") -> None:
        """Provision a board with the given configuration."""
        node = self.get_node(node_id)

        # Save to temporary file for validation
        fd, path = tempfile.mkstemp(suffix=f".{config_type}", prefix=f"virtmcu-{node_id}-")
        os.write(fd, board_config.encode("utf-8"))
        os.close(fd)

        # Validate by trying to generate DTB
        dtb_fd, dtb_path = tempfile.mkstemp(suffix=".dtb")
        os.close(dtb_fd)

        f_out = io.StringIO()
        f_err = io.StringIO()

        try:
            with redirect_stdout(f_out), redirect_stderr(f_err):
                if config_type == "yaml":
                    from virtmcu_tools.yaml2qemu import main as yaml2qemu_main

                    old_argv = sys.argv
                    sys.argv = ["yaml2qemu", "--out-dtb", dtb_path, path]
                    try:
                        yaml2qemu_main()
                    except SystemExit as e:
                        if e.code != 0:
                            raise ValueError(f"yaml2qemu failed with code {e.code}: {f_err.getvalue()}") from e
                    finally:
                        sys.argv = old_argv
                else:
                    # REPL validation
                    from virtmcu_tools.repl2qemu.__main__ import main as repl2qemu_main

                    old_argv = sys.argv
                    sys.argv = ["repl2qemu", path, "--out-dtb", dtb_path]
                    try:
                        repl2qemu_main()
                    except SystemExit as e:
                        if e.code != 0:
                            raise ValueError(f"repl2qemu failed with code {e.code}: {f_err.getvalue()}") from e
                    finally:
                        sys.argv = old_argv
        except (Exception, BaseException) as e:
            if Path(path).exists():
                Path(path).unlink()
            if Path(dtb_path).exists():
                Path(dtb_path).unlink()
            # Log the captured output for debugging
            logger.error(f"Validation failed. stdout: {f_out.getvalue()} stderr: {f_err.getvalue()}")
            raise ValueError(f"Invalid board configuration: {e}") from e
        finally:
            if Path(dtb_path).exists():
                Path(dtb_path).unlink()

        if node.yaml_path and Path(node.yaml_path).exists():
            Path(node.yaml_path).unlink()
        node.yaml_path = path

    def flash_firmware(self, node_id: str, firmware_path: str) -> None:
        """Flash firmware to the given node."""
        node = self.get_node(node_id)
        f_path = Path(firmware_path)
        if not f_path.is_absolute():
            f_path = f_path.resolve()
        if not f_path.exists():
            raise FileNotFoundError(f"Firmware file not found: {f_path}")
        node.firmware_path = f_path

    async def start_node(self, node_id: str) -> None:
        """Start the QEMU process for the given node."""
        node = self.get_node(node_id)
        if node.process and node.process.returncode is None:
            raise RuntimeError(f"Node {node_id} is already running.")

        if not node.yaml_path:
            raise RuntimeError(f"Node {node_id} has not been provisioned.")

        # Clean up any stale sockets
        if Path(node.qmp_socket_path).exists():
            Path(node.qmp_socket_path).unlink()
        if Path(node.uart_socket_path).exists():
            Path(node.uart_socket_path).unlink()

        cmd = [
            "bash",
            "scripts/run.sh",
            f"--{node.yaml_path.split('.')[-1]}",
            node.yaml_path,
        ]

        if node.firmware_path:
            cmd.extend(["--kernel", str(node.firmware_path)])

        # Add QMP and UART sockets
        cmd.extend(
            [
                "-qmp",
                f"unix:{node.qmp_socket_path},server,nowait",
                "-serial",
                f"unix:{node.uart_socket_path},server,nowait",
                "-nographic",
            ]
        )

        logger.info(f"Starting node {node_id} with cmd: {' '.join(cmd)}")
        node.process = await asyncio.create_subprocess_exec(
            *cmd, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE
        )

        from virtmcu_tools.utils import wait_for_file_creation, yield_now

        # Wait for sockets deterministically
        try:
            wait_tasks = [
                wait_for_file_creation(node.qmp_socket_path),
                wait_for_file_creation(node.uart_socket_path),
            ]

            files_task = asyncio.ensure_future(asyncio.gather(*wait_tasks))
            exit_task = asyncio.create_task(node.process.wait())

            done, pending = await asyncio.wait(
                [files_task, exit_task],
                return_when=asyncio.FIRST_COMPLETED,
                timeout=10.0,
            )

            for task in pending:
                task.cancel()
                with suppress(asyncio.CancelledError):
                    await task

            if exit_task in done:
                await yield_now()
                stderr_data = b""
                if node.process.stderr:
                    stderr_data = await node.process.stderr.read()
                raise RuntimeError(
                    f"QEMU process exited early with code {node.process.returncode}: {stderr_data.decode()}"
                )

            if not done:
                raise TimeoutError()

            files_task.result()

        except (TimeoutError, RuntimeError) as e:
            if node.process.returncode is None:
                node.process.terminate()
            stderr_data = b""
            if node.process.stderr:
                stderr_data = await node.process.stderr.read()
            logger.error(f"QEMU failed to start. STDERR: {stderr_data.decode()}")
            if isinstance(e, TimeoutError):
                raise RuntimeError(f"QEMU QMP/UART sockets did not appear in time for {node_id}") from e
            raise

        try:
            import re

            match = re.search(r"node(\d+)", node_id)
            n_id = int(match.group(1)) if match else None

            await node.qmp_bridge.connect(
                node.qmp_socket_path,
                node.uart_socket_path,
                zenoh_session=self.get_zenoh_session(),
                node_id=n_id,
            )
        except Exception as e:
            if node.process.returncode is None:
                node.process.terminate()
            if node.process.stderr:
                stderr_b = await node.process.stderr.read()
                raise RuntimeError(f"QMP connection failed: {e}. QEMU stderr: {stderr_b.decode()}") from e
            raise RuntimeError(f"QMP connection failed: {e}") from e

    async def stop_node(self, node_id: str) -> None:
        """Stop the QEMU process for the given node and clean up."""
        if node_id not in self.nodes:
            return
        node = self.nodes[node_id]
        if node.process and node.process.returncode is None:
            node.process.terminate()
            try:
                await asyncio.wait_for(node.process.wait(), timeout=5.0)
            except TimeoutError:
                node.process.kill()
                await node.process.wait()

        await node.qmp_bridge.close()

        for path in [node.qmp_socket_path, node.uart_socket_path]:
            if path and Path(path).exists():
                with suppress(OSError):
                    Path(path).unlink()
