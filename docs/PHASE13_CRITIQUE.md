# Phase 13 Critique: AI Debugging & MCP Interface

## Findings & Fixes

### 1. JSON-RPC Corruption during Validation
**Issue**: Calling `yaml2qemu.main()` directly for validation caused it to print to `stdout`, which corrupted the MCP JSON-RPC stream and crashed the client/server communication.
**Fix**: Redirected `stdout` to `stderr` during validation in `NodeManager.provision_board`.

### 2. Lack of Early Validation
**Issue**: `provision_board` was only saving the YAML/REPL file without checking if it was valid. Errors were only caught later during `start_node`.
**Fix**: Integrated `yaml2qemu` and `repl2qemu` parsers into `provision_board` to validate configurations before saving.

### 3. Inefficient Zenoh Session Management
**Issue**: `set_network_latency` was opening a new Zenoh session for every call, leading to significant overhead.
**Fix**: Implemented a shared, lazy-initialized Zenoh session in `NodeManager`.

### 4. Safety & Stability
**Issue**: `read_memory` could potentially read massive amounts of memory, risking OOM on the host.
**Fix**: Added a 1MB safety limit on memory read sizes.
**Issue**: `SystemExit` from CLI tools (like `argparse` or `main` functions) was crashing the whole MCP server.
**Fix**: Added `SystemExit` to the exception catch block in `provision_board`.

### 5. Multi-Node Support
**Issue**: No integration tests covered multi-node scenarios via MCP.
**Fix**: Added `test/phase13/multi_node_mcp_test.py`.

## Test Coverage Improvements
- Added `test/phase13/mcp_stress_test.py` for rapid start/stop stability.
- Added `test/phase13/validation_test.py` for config validation verification.
- Increased `tools/mcp_server/server.py` coverage from ~58% to **91%**.
- Increased `tools/mcp_server/node_manager.py` coverage from ~58% to **69%** (limited by QEMU process mocks).
- Overall Phase 13 coverage increased from **54%** to **74%**.

## Future Recommendations
- **Structured CPU State**: Migrate from `info registers` string to a more structured register retrieval if QMP supports it.
- **Interrupt Injection**: Investigate target-specific IRQ injection via `qom-set`.
- **GDB Integration**: Allow the MCP server to manage GDB sessions for deeper debugging.
