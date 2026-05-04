import concurrent.futures
import os
from collections.abc import Callable
from pathlib import Path

from tools.testing.env import WORKSPACE_DIR

SCRIPT_PATH = WORKSPACE_DIR / "scripts" / "get-free-port.py"


def test_single_allocation(script_runner: Callable[..., str]) -> None:
    """Ensure it can allocate a single port correctly."""
    output = script_runner(SCRIPT_PATH, "--port")
    port = int(output)
    assert 1024 <= port <= 65535


def test_concurrent_allocations(tmp_path: Path, script_runner: Callable[..., str]) -> None:
    """
    Ensure multiple parallel requests don't receive the same port.
    Overrides the reservation dir to a pytest tmp_path to ensure isolation.
    """
    env = os.environ.copy()
    env["VIRTMCU_PORT_RESERVATION_DIR"] = str(tmp_path)

    def get_port() -> int:
        output = script_runner(SCRIPT_PATH, "--port", env=env)
        return int(output)

    n_requests = 50
    # Simulate heavy parallel usage
    with concurrent.futures.ThreadPoolExecutor(max_workers=10) as executor:
        ports = list(executor.map(lambda _: get_port(), range(n_requests)))

    assert len(ports) == n_requests
    duplicates = {p for p in ports if ports.count(p) > 1}
    assert not duplicates, f"Found duplicate ports: {duplicates}"


def test_ip_endpoint_formats(script_runner: Callable[..., str]) -> None:
    """Ensure the different argument flags return expected formats."""
    ip_out = script_runner(SCRIPT_PATH, "--ip")
    assert "." in ip_out

    ep_out = script_runner(SCRIPT_PATH, "--endpoint", "--proto", "tcp/")
    assert ep_out.startswith("tcp/")
    assert ":" in ep_out
