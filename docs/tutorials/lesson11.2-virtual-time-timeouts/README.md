# Lesson 11.2: Virtual-Time-Aware Timeouts

## The Problem with Wall-Clock Timeouts

When QEMU runs in `slaved-icount` mode (`-icount shift=0,align=off,sleep=off`), it
executes at roughly 15–20% of normal speed.  The emulator advances one virtual nanosecond
per instruction, which gives firmware deterministic access to timers and delays, but means
a test that expects a 100 ms boot message might wait 500–700 ms of *wall time* before it
arrives.

A naive timeout like `asyncio.wait_for(wait_for_uart(), timeout=0.5)` will fire before
the firmware has had a chance to output anything.  The test fails — not because the
firmware is wrong, but because the test's clock is running at a different speed from the
VM's clock.

## The Fix: Use the VM's Own Clock

QEMU exposes the current instruction count through the `query-replay` QMP command:

```json
→ { "execute": "query-replay" }
← { "return": { "mode": "none", "icount": 1234567 } }
```

The `icount` field is always present, regardless of whether the full record/replay
subsystem is active.  In `slaved-icount` mode with `shift=0`, the relationship is exact:

```
1 instruction == 1 virtual nanosecond
icount (instructions) == virtual_time_ns
```

So if `timeout = 10.0` seconds, the test should fire when:

```
(icount_now - icount_start) / 1e9 > 10.0
```

## How `QmpBridge` Implements This

`tools/testing/qmp_bridge.py` provides `get_virtual_time_ns()`:

```python
async def get_virtual_time_ns(self) -> int:
    try:
        res = await self.execute("query-replay")
        return res.get("icount", 0)
    except Exception:
        return 0   # QMP not ready yet — callers will fall back to wall clock
```

`wait_for_line_on_uart` and `wait_for_event` both sample this value at the start and
on every polling iteration, then choose their timeout source:

```python
start_vtime = await self.get_virtual_time_ns()
start_wall  = loop.time()

# … in the polling loop …
current_vtime = await self.get_virtual_time_ns()
if current_vtime > start_vtime:
    # Virtual time is advancing → use it (slaved-icount mode)
    if (current_vtime - start_vtime) / 1e9 > timeout:
        return False   # or raise TimeoutError
else:
    # Virtual time stuck at 0 → use wall clock (standalone mode)
    if loop.time() - start_wall > timeout:
        return False
```

## Mode Detection is Automatic

There is no configuration flag.  The two modes are detected at runtime:

| Mode | `query-replay` returns | Timeout source |
|------|------------------------|----------------|
| Standalone (no `-icount`) | `icount: 0` (never advances) | wall clock |
| `slaved-icount shift=0` | icount increments each call | virtual time |

This means the same test suite works in both CI (standalone, fast) and FirmwareStudio
integration (slaved-icount, ~15–20% speed) without any changes.

## Verifying the Behaviour

Run the unit tests that exercise the timeout logic without a live QEMU:

```bash
python3 -m pytest tests/test_qmp_bridge.py -v
```

You should see tests such as:

- `test_wait_for_line_wall_clock_timeout` — confirms that when `icount` stays 0,
  the wall-clock fallback fires within the specified window.
- `test_wait_for_line_virtual_time_timeout` — confirms that a large icount jump
  triggers the virtual-time timeout even if wall time has barely elapsed.
- `test_wait_for_event_virtual_time_timeout` — same check for the event-wait path.

## Practical Guidance

When writing tests that boot firmware:

- Use `timeout=10.0` (seconds) as a rule of thumb.  In standalone mode this is 10
  wall-clock seconds; in slaved-icount mode this is 10 *virtual* seconds, which may
  correspond to 50–70 wall-clock seconds depending on host load.
- Do not manually call `asyncio.sleep()` for timing inside tests — use `wait_for_line_on_uart`
  with an appropriate pattern instead.
- If you need to assert that something happens within a *virtual* time budget (e.g.
  "the RTOS task must run within 1 ms of virtual time"), pass `timeout=0.001`.
