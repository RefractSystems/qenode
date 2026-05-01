from __future__ import annotations

import logging
from typing import TYPE_CHECKING

import gdb

if TYPE_CHECKING:
    pass

logger = logging.getLogger(__name__)

# ==============================================================================
# QEMU / QOM Breakpoint Helpers
# ==============================================================================


class TraceBreakpoint(gdb.Breakpoint):  # type: ignore[misc]
    """
    A generic GDB breakpoint that prints a custom message and continues execution.
    Useful for tracing device realization, interrupts, etc.
    """

    def __init__(self, spec: str, message: str) -> None:
        super().__init__(spec, gdb.BP_BREAKPOINT, internal=False)
        self.message = message
        # Make the breakpoint silent by default so we can control output
        self.silent = True

    def stop(self) -> bool:
        """Called when the breakpoint is hit. Return False to continue execution."""
        # We can extract the PC or arguments if needed
        frame = gdb.newest_frame()
        pc = frame.pc()
        logger.info(f"[GDB-TRACE] {hex(pc)}: {self.message}")

        # Continue execution
        return False


class QOMTraceDeviceRealize(TraceBreakpoint):
    """
    Traces QEMU's `device_set_realized` to show which devices are actually initializing.
    """

    def __init__(self) -> None:
        super().__init__("device_set_realized", "Realizing QOM Device")

    def stop(self) -> bool:
        try:
            # device_set_realized takes (Object *obj, bool value, Error **errp)
            frame = gdb.newest_frame()
            obj_val = frame.read_var("obj")

            # Try to cast to Object* to read the class type name
            obj_type = gdb.lookup_type("Object").pointer()
            obj_ptr = obj_val.cast(obj_type)
            class_ptr = obj_ptr["class"]

            type_ptr = class_ptr["type"]
            type_name = type_ptr["name"].string()

            logger.info(f"[QOM-REALIZE] Instantiating: {type_name}")
        except (gdb.error, AttributeError, TypeError) as e:
            logger.info(f"[QOM-REALIZE] Realize called, but failed to parse arguments: {e}")

        return False


def setup_tracing() -> None:
    """Sets up the default VirtMCU trace points."""
    logger.info("Setting up VirtMCU GDB tracing...")
    QOMTraceDeviceRealize()


def trace_function(func_name: str) -> None:
    """Creates a simple trace breakpoint for a given function."""
    TraceBreakpoint(func_name, f"{func_name} called")
    logger.info(f"Tracing enabled for {func_name}")


logger.info("VirtMCU GDB helpers loaded. Available functions:")
logger.info("  setup_tracing()      - Trace QOM device realizations")
logger.info("  trace_function(name) - Create a trace point for any function")
