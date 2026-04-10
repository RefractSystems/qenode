# Lesson 4: Emulation Test Automation with QMP and Pytest

In this lesson, you will learn how to automate the testing of your firmware and virtual hardware using the **QEMU Machine Protocol (QMP)**. We will explore how to use the `QmpBridge` to control QEMU, monitor UART output, and assert internal CPU state.

## Concepts

### QEMU Machine Protocol (QMP)
QMP is a JSON-based protocol that allows you to control QEMU while it's running. Unlike the human monitor (HMP), QMP is designed for machines. It allows you to:
- Pause and resume the CPU (`stop`, `cont`).
- Reset the system (`system_reset`).
- Query status, CPU registers, and the object model.
- Receive asynchronous events (e.g., `SHUTDOWN`, `RESET`).

### UART Monitoring via Chardev Sockets
To verify that firmware is "alive," we often look for specific strings on the serial port. In `virtmcu`, we configure QEMU to expose its UART as a **Unix Domain Socket**. This allows our Python test scripts to connect as a client and read the byte stream without interfering with the host's terminal.

---

## The QmpBridge

We have provided a utility called `QmpBridge` in `tools/testing/qmp_bridge.py`. It is an asynchronous Python class that wraps the `qemu.qmp` library.

### Key Methods:
- `connect(qmp_sock, uart_sock)`: Opens connections to both sockets.
- `start_emulation()` / `pause_emulation()`: Controls the TCG execution loop.
- `wait_for_line_on_uart(pattern, timeout)`: Scans the incoming byte stream for a regex match.
- `get_pc()`: Retrieves the current Program Counter (PC/R15).

---

## Hands-on: Writing a Pytest

Pytest is our primary testing framework. We use fixtures to manage the lifecycle of the QEMU process.

### 1. The Fixtures (`conftest.py`)
We use a fixture called `qmp_bridge` which:
1. Starts QEMU in a paused state (`-S`).
2. Connects the `QmpBridge`.
3. Resumes emulation.
4. Cleans up (kills QEMU) when the test is done.

### 2. The Test Case (`test_my_firmware.py`)

```python
import pytest

@pytest.mark.asyncio
async def test_hello_world(qmp_bridge):
    # 1. Wait for the firmware to print "HI"
    found = await qmp_bridge.wait_for_line_on_uart("HI", timeout=5.0)
    assert found, "Firmware did not print HI"
    
    # 2. Check the Program Counter
    pc = await qmp_bridge.get_pc()
    assert pc >= 0x40000000
```

---

## Hands-on: Robot Framework Compatibility

For users coming from **Renode**, we provide a Robot Framework keyword library that maps 1:1 to common Renode keywords.

### Example Robot Test:

```robot
*** Settings ***
Resource    ../../tools/testing/qemu_keywords.robot
Test Teardown   Terminate Emulation

*** Test Cases ***
Should Print HI
    ${qmp}    ${uart}=    Launch Qemu    my_board.dtb    hello.elf    extra_args=-S
    Connect To Emulation    ${qmp}    ${uart}
    Start Emulation
    Wait For Line On UART    HI
```

---

## Exercise: Debugging a Crash

If your firmware crashes (e.g., executes an invalid instruction), QEMU might enter a `guest-panicked` state or just loop at a specific address.

1. Launch the Phase 1 `hello.elf`.
2. Use `await qmp_bridge.get_pc()` in a loop to see it spinning in the `wfi` loop.
3. Try sending `system_reset` via the bridge and verify the PC returns to the entry point (`0x40000000`).

## Summary
- **QMP** is the programmatic "remote control" for QEMU.
- **Unix Sockets** are used for both control (QMP) and data (UART).
- **Automation** ensures that as you add new peripherals (Phase 2) or change your platform (Phase 3), your firmware continues to behave correctly.
