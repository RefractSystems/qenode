import asyncio
import logging
import os
import tempfile
from typing import Dict, Optional

from tools.testing.qmp_bridge import QmpBridge

logger = logging.getLogger(__name__)

class NodeContext:
    def __init__(self, node_id: str):
        self.node_id = node_id
        self.process: Optional[asyncio.subprocess.Process] = None
        self.qmp_bridge = QmpBridge()
        self.qmp_socket_path = f"/tmp/virtmcu-{node_id}.qmp"
        self.uart_socket_path = f"/tmp/virtmcu-{node_id}.uart"
        self.yaml_path: Optional[str] = None
        self.firmware_path: Optional[str] = None

class NodeManager:
    def __init__(self):
        self.nodes: Dict[str, NodeContext] = {}

    def get_node(self, node_id: str) -> NodeContext:
        if node_id not in self.nodes:
            self.nodes[node_id] = NodeContext(node_id)
        return self.nodes[node_id]

    async def provision_board(self, node_id: str, board_config: str, config_type: str = "yaml"):
        node = self.get_node(node_id)
        if node.yaml_path and os.path.exists(node.yaml_path):
            os.remove(node.yaml_path)
            
        fd, path = tempfile.mkstemp(suffix=f".{config_type}", prefix=f"virtmcu-{node_id}-")
        os.write(fd, board_config.encode('utf-8'))
        os.close(fd)
        node.yaml_path = path
        
    def flash_firmware(self, node_id: str, firmware_path: str):
        node = self.get_node(node_id)
        if not os.path.isabs(firmware_path):
            firmware_path = os.path.abspath(firmware_path)
        if not os.path.exists(firmware_path):
            raise FileNotFoundError(f"Firmware file not found: {firmware_path}")
        node.firmware_path = firmware_path

    async def start_node(self, node_id: str):
        node = self.get_node(node_id)
        if node.process and node.process.returncode is None:
            raise RuntimeError(f"Node {node_id} is already running.")
            
        if not node.yaml_path:
            raise RuntimeError(f"Node {node_id} has not been provisioned.")
            
        cmd = [
            "bash", "scripts/run.sh",
            f"--{node.yaml_path.split('.')[-1]}", node.yaml_path,
        ]
        
        if node.firmware_path:
            cmd.extend(["--kernel", node.firmware_path])
            
        # Add QMP and UART sockets
        cmd.extend([
            "-qmp", f"unix:{node.qmp_socket_path},server,nowait",
            "-serial", f"unix:{node.uart_socket_path},server,nowait",
            "-nographic"
        ])

        logger.info(f"Starting node {node_id} with cmd: {' '.join(cmd)}")
        node.process = await asyncio.create_subprocess_exec(
            *cmd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE
        )
        
        # Wait a bit for QEMU to create the sockets
        for _ in range(50):
            if os.path.exists(node.qmp_socket_path):
                break
            await asyncio.sleep(0.1)
            
        if not os.path.exists(node.qmp_socket_path):
            raise RuntimeError(f"QEMU failed to start or create QMP socket for {node_id}")

        await node.qmp_bridge.connect(node.qmp_socket_path, node.uart_socket_path)

    async def stop_node(self, node_id: str):
        if node_id not in self.nodes:
            return
        node = self.nodes[node_id]
        if node.process and node.process.returncode is None:
            node.process.terminate()
            try:
                await asyncio.wait_for(node.process.wait(), timeout=5.0)
            except asyncio.TimeoutError:
                node.process.kill()
                await node.process.wait()
        
        await node.qmp_bridge.close()
        
        for path in [node.qmp_socket_path, node.uart_socket_path, node.yaml_path]:
            if path and os.path.exists(path):
                try:
                    os.remove(path)
                except OSError:
                    pass
