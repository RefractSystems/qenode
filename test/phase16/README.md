# Phase 16: Performance & Determinism CI

This directory contains a benchmarking suite for `virtmcu`.

## Key Metrics Measured
- **IPS (Instructions Per Second)**: Raw emulation speed in MIPS.
- **Determinism**: Verification that instruction counts are identical across multiple slaved-icount runs.
- **Co-simulation Latency**: Round-trip overhead for Zenoh-based clock synchronization.

## Benchmarking Results (Typical)
| Mode | Speed (MIPS) | Deterministic? | Latency (RT) |
|---|---|---|---|
| `standalone` | ~2700 | No | N/A |
| `slaved-icount` | ~1100 | **Yes** | ~0.25 ms |

## Usage
Run the automated test:
```bash
./smoke_test.sh
```

Or run the python script directly:
```bash
python3 bench.py
```
