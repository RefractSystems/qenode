import asyncio
import io
import logging
import os
import sys
import tempfile
from contextlib import redirect_stderr, redirect_stdout, suppress
from pathlib import Path

from tools.testing.qmp_bridge import QmpBridge

logger = logging.getLogger(__name__)


class NodeContext:
    def __init__(self, node_id: str, base_tmpdir: Path):
        self.node_id = node_id
        self.process: asyncio.subprocess.Process | None = None
        self.qmp_bridge = QmpBridge()
        self.qmp_socket_path = str(base_tmpdir / f"virtmcu-{node_id}.qmp")
        self.uart_socket_path = str(base_tmpdir / f"virtmcu-{node_id}.uart")
        self.yaml_path: str | None = None
        self.firmware_path: str | None = None


class NodeManager:
    def __init__(self):
        self.nodes: dict[str, NodeContext] = {}
        self._zenoh_session = None
        self.base_tmpdir = Path(tempfile.mkdtemp(prefix="virtmcu-mcp-"))

    def get_zenoh_session(self):
        import zenoh

        if self._zenoh_session is None:
            self._zenoh_session = zenoh.open(zenoh.Config())
        return self._zenoh_session

    async def close(self):
        for node in self.nodes.values():
            await self.stop_node(node.node_id)
        if self._zenoh_session:
            self._zenoh_session.close()
            self._zenoh_session = None

        # Cleanup base tmpdir
        import shutil

        shutil.rmtree(self.base_tmpdir, ignore_errors=True)

    def get_node(self, node_id: str) -> NodeContext:
        if node_id not in self.nodes:
            self.nodes[node_id] = NodeContext(node_id, self.base_tmpdir)
        return self.nodes[node_id]

    async def provision_board(self, node_id: str, board_config: str, config_type: str = "yaml"):
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
                    from tools.yaml2qemu import main as yaml2qemu_main

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
                    from tools.repl2qemu.__main__ import main as repl2qemu_main

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

    def flash_firmware(self, node_id: str, firmware_path: str):
        node = self.get_node(node_id)
        if not Path(firmware_path).is_absolute():
            firmware_path = str(Path(firmware_path).resolve())
        if not Path(firmware_path).exists():
            raise FileNotFoundError(f"Firmware file not found: {firmware_path}")
        node.firmware_path = firmware_path

    async def start_node(self, node_id: str):
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
            cmd.extend(["--kernel", node.firmware_path])

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

        # Wait a bit for QEMU to create the sockets
        for _ in range(50):
            if Path(node.qmp_socket_path).exists() and Path(node.uart_socket_path).exists():
                break
            # Check if process exited early
            if node.process and node.process.returncode is not None:
                stderr_data = b""
                if node.process.stderr:
                    stderr_data = await node.process.stderr.read()
                raise RuntimeError(
                    f"QEMU process exited early with code {node.process.returncode}: {stderr_data.decode()}"
                )
            await asyncio.sleep(0.1)

        if not Path(node.qmp_socket_path).exists():
            if node.process and node.process.returncode is None:
                node.process.terminate()
                stderr_data = b""
                if node.process.stderr:
                    stderr_data = await node.process.stderr.read()
                raise RuntimeError(f"QEMU failed to create QMP socket for {node_id}. stderr: {stderr_data.decode()}")
            raise RuntimeError(f"QEMU failed to start or create QMP socket for {node_id}")

        try:
            await node.qmp_bridge.connect(node.qmp_socket_path, node.uart_socket_path)
        except Exception as e:
            if node.process and node.process.returncode is None:
                node.process.terminate()
            stderr_data = b""
            if node.process and node.process.stderr:
                stderr_data = await node.process.stderr.read()
            raise RuntimeError(f"QMP connection failed: {e}. QEMU stderr: {stderr_data.decode()}") from e

    async def stop_node(self, node_id: str):
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

        for path in [node.qmp_socket_path, node.uart_socket_path, node.yaml_path]:
            if path and Path(path).exists():
                with suppress(OSError):
                    Path(path).unlink()
