"""
SOTA Enterprise Grade Test Parameter Management.

Centralizes environment-aware scaling for timeouts and iteration counts.
Eliminates "whack-a-mole" ASan tuning and prevents double-scaling bugs.
"""

import logging
import os

logger = logging.getLogger(__name__)


class TestParams:
    @staticmethod
    def multiplier() -> float:
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

    @staticmethod
    def scale_timeout(logical_seconds: float) -> float:
        """
        Scales a logical timeout (in seconds) based on the current environment.

        Use this for raw `asyncio.sleep` or `asyncio.wait_for`.
        Do NOT use this for framework methods (like `vta.step` or `run_until`)
        that already scale their `timeout=` parameters internally!
        """
        if logical_seconds >= 100.0:
            logger.error(
                f"SOTA ERROR: Very large logical timeout ({logical_seconds}s) passed. "
                "Ensure you are not passing an already-scaled value (double-scaling)."
            )
        return logical_seconds * TestParams.multiplier()

    @staticmethod
    def scale_iters(logical_count: int) -> int:
        """
        Scales an iteration count based on the current environment.
        Useful for polling loops or stress test bounds.
        """
        return int(max(1, int(logical_count * TestParams.multiplier())))

    @staticmethod
    def get_stall_timeout_ms(base_ms: int | None = None) -> int:
        """Gets the scaled stall-timeout for QEMU virtual clocks."""
        base = base_ms if base_ms is not None else int(os.environ.get("VIRTMCU_STALL_TIMEOUT_MS", "5000"))
        return int(base * TestParams.multiplier())
