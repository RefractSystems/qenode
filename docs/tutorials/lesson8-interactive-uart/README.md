# Lesson 8: Interactive UART and Multi-Node Deterministic Serial

In this lesson, we explore how to interact directly with a running `virtmcu` instance using serial (UART) communication, and how to extend that capability across multiple nodes deterministically.

## 1. The Interactive Echo Firmware

Up until now, our test firmware (like `hello.elf`) has simply printed a message and halted. In a real development scenario, you often need interactive access to the firmware (e.g., a CLI shell, debugging menu, or sensor polling).

We have developed a basic "Interactive UART Echo" firmware in `tests/fixtures/guest_apps/uart_echo/echo.S`. 

### Key Concepts:
1. **Hardware Initialization:** Before a bare-metal program can print, it must enable the UART peripheral by writing to its Control Register (e.g., setting the `UARTEN`, `TXE`, and `RXE` bits on the ARM PL011).
2. **Polling Loop:** The firmware continuously polls the UART Flag Register (`UART0_FR`) checking the Receive FIFO Empty (`RXFE`) bit. When data arrives, it reads the Data Register (`UART0_DR`) and writes it back to echo the character to the user.

### Running it Manually

You can boot this firmware and interact with it directly in your terminal using standard QEMU arguments.

```bash
# Compile the firmware
make -C tests/fixtures/guest_apps/uart_echo

# Run virtmcu with stdio mapped to the primary serial port
./scripts/run.sh --dtb tests/fixtures/guest_apps/boot_arm/minimal.dtb --kernel tests/fixtures/guest_apps/uart_echo/echo.elf -nographic
```

Once booted, you will see:
```
Interactive UART Echo Ready.
Type something: 
```
Any keys you press will be echoed back immediately by the emulated Cortex-A15 processor.

*(Press `Ctrl-A` then `x` to exit QEMU).*

## 2. Automated Testing of Interactive Firmware

To prevent regressions, interactive firmware must be tested using an automated harness. The virtmcu project provides a Python `Simulation` fixture in `pytest` to achieve this.

### The `simulation` fixture and `sim.transport`

In the UART echo tests, we utilize the `simulation` fixture. This allows the test script to simulate a user typing into the console and reading the output deterministically.

Here is an example test concept from `tests/integration/simulation/peripherals/test_uart_echo.py`:

```python
@pytest.mark.asyncio
async def test_interactive_echo(simulation):
    # Setup node with the echo firmware
    simulation.add_node(node_id=0, dtb=dtb_path, kernel=kernel_path, extra_args=[
        "-chardev", "virtmcu,id=char0,topic=virtmcu/uart", "-serial", "chardev:char0"
    ])
    
    async with simulation as sim:
        # 1. Wait for welcome message
        assert await sim.bridge.wait_for_line_on_uart("Interactive UART Echo Ready.", timeout=5.0)
        
        # 2. Simulate user typing
        header = vproto.ZenohFrameHeader(vtime_ns, sequence, length).pack()
        await sim.transport.publish("virtmcu/uart/0/rx", header + b"Hello virtmcu\r")
        
        # 3. Verify the firmware echoed it back
        assert await sim.bridge.wait_for_line_on_uart("Hello virtmcu", timeout=5.0)
```

Notice how we boot QEMU using the `Simulation` context manager (`async with simulation as sim:`).
This is critical. The framework automatically injects the `-S` (suspend) flag during boot, preventing the firmware from executing instantly and printing the welcome message to the socket *before* our test harness has connected to read it. By pausing QEMU, connecting our listeners, and *then* issuing the start command via QMP `cont`, we guarantee no data is lost.

## 3. Multi-Node Deterministic UART (Upcoming)

While interactive local debugging is useful, true system validation often requires multiple microcontrollers communicating with each other over serial connections (e.g., an MCU commanding a GPS module or a motor driver over UART).

In the next stage, we will implement `chardev.c`. This native QEMU plugin will allow us to map the UART byte stream to Zenoh topics (e.g., `virtmcu/uart/node1/tx`), enabling cycle-accurate, deterministic serial communication between multiple emulated nodes across the network, just as we did with Ethernet in earlier coordinator tests.

## 4. Running the Multi-Node Deterministic UART

Now that the basic UART is complete, you can map the emulated UART directly to the Zenoh network. This allows Node 1's UART to appear as incoming UART data on Node 2, completely deterministically.

1. **Start the Zenoh Coordinator:**
   ```bash
   export PATH="$HOME/.cargo/bin:$PATH"
   cd tools/deterministic_coordinator
   cargo run -- --delay-ns 500000
   ```

2. **Start Node 1 (in a new terminal):**
   ```bash
   ./scripts/run.sh --dtb tests/fixtures/guest_apps/boot_arm/minimal.dtb --kernel tests/fixtures/guest_apps/uart_echo/echo.elf -nographic \
       -chardev zenoh,id=chr0,node=node1 \
       -serial chardev:chr0
   ```

3. **Start Node 2 (in a new terminal):**
   ```bash
   ./scripts/run.sh --dtb tests/fixtures/guest_apps/boot_arm/minimal.dtb --kernel tests/fixtures/guest_apps/uart_echo/echo.elf -nographic \
       -chardev zenoh,id=chr0,node=node2 \
       -serial chardev:chr0
   ```

Any characters typed into the terminal for Node 1 will be timestamped, published to Zenoh, routed by the coordinator, and deterministically injected into Node 2's UART receiver exactly `500,000` virtual nanoseconds later.
