import logging

import gdb

logger = logging.getLogger(__name__)


class TraceBreakpoint(gdb.Breakpoint):
    """
    A generic breakpoint that prints a message and continues execution.
    Useful for tracing without stopping the entire QEMU instance.
    """

    def __init__(self, spec, message):
        super().__init__(spec, gdb.BP_BREAKPOINT, internal=False)
        self.message = message

    def stop(self):
        logger.info(f"TRACE: {self.message}")
        return False  # Return False to continue execution


class QOMTraceDeviceRealize(gdb.Breakpoint):
    """
    A breakpoint specifically for tracing QOM device realization.
    It automatically prints the device ID and canonical path.
    """

    def __init__(self):
        super().__init__("fdt_init_device_realize", gdb.BP_BREAKPOINT, internal=False)

    def stop(self):
        try:
            # Assuming 'dev' is a local variable in the frame of fdt_init_device_realize
            dev_id = gdb.parse_and_eval("(char*)(DEVICE(dev))->id")
            path = gdb.parse_and_eval("(char*)object_get_canonical_path(dev)")
            logger.info(f"Realizing device: ID={dev_id.string()}, Path={path.string()}")

            # Note: object_get_canonical_path allocates memory that should be freed,
            # but for a quick debug script we might leak it or handle it carefully.
            # A safer approach for gdb is just to read the properties directly if possible.
        except Exception as e:
            logger.error(f"Error reading device info: {e}")

        return False


# Register custom commands or helpers here
def setup_tracing():
    """Example helper to set up common traces."""
    logger.info("Setting up QOM device realization trace...")
    QOMTraceDeviceRealize()


def trace_function(func_name):
    """Creates a simple trace breakpoint for a given function."""
    TraceBreakpoint(func_name, f"{func_name} called")
    logger.info(f"Tracing enabled for {func_name}")


logger.info("VirtMCU GDB helpers loaded. Available functions:")
logger.info("  setup_tracing()      - Trace QOM device realizations")
logger.info("  trace_function(name) - Create a trace point for any function")
