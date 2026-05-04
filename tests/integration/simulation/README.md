# Simulation Tests

This directory contains integration tests whose System-Under-Test (SUT) is the **guest firmware** or the **QEMU peripheral models** (e.g., UART, LIN, FlexRay, Actuators).

## Transport Agnosticism Mandate

By definition, these tests treat the networking layer as a "dumb pipe". They **MUST NOT** directly interact with Zenoh.

- **BANNED**: `import zenoh`
- **BANNED**: `zenoh_session.put()`, `zenoh_session.declare_subscriber()`
- **REQUIRED**: All network I/O must be performed via `simulation.transport.publish()` and `simulation.transport.subscribe()`.

This ensures the firmware can be tested seamlessly across standard Zenoh networks, high-performance Unix domain sockets, or specialized Chaos-engineering fault injectors.
