from __future__ import annotations

from typing import TYPE_CHECKING, Protocol, runtime_checkable

if TYPE_CHECKING:
    from pathlib import Path

    from tools.testing.virtmcu_test_suite.conftest_core import VirtmcuSimulation


@runtime_checkable
class SimulationCreator(Protocol):
    async def __call__(
        self,
        dtb_path: str | Path,
        kernel_path: str | Path | None = None,
        nodes: list[int] | None = None,
        extra_args: list[str] | None = None,
        **kwargs: object,
    ) -> VirtmcuSimulation: ...
