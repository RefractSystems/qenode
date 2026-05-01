# Lesson 13: AI-Augmented Debugging

## Introduction

Welcome to Lesson 13! As simulations become more complex—especially in multi-node, cyber-physical environments—debugging firmware crashes like stack overflows or race conditions using traditional CLI tools (like GDB and QMP) becomes unwieldy.

In this lesson, you will learn how to use the Model Context Protocol (MCP) server provided by virtmcu. The MCP server allows AI assistants (like Claude Desktop or Gemini CLI) to semantically interact with your simulation environment.

## What is MCP?

The Model Context Protocol (MCP) is an open standard that allows AI models to securely access external data sources and tools. By implementing an MCP server, virtmcu exposes its low-level QEMU and Zenoh capabilities as high-level "Tools" and "Resources" that an AI can understand and invoke.

## Starting the MCP Server

The virtmcu MCP server is written in Python and is located in `tools/mcp_server`. To use it with an AI client, you configure the client to launch the server as a subprocess.

```bash
python3 -m virtmcu.mcp_server
```

## Available Tools

Once connected, your AI assistant can use the following tools:

- **provision_board**: Provide a YAML description to create a virtual machine layout.
- **flash_firmware**: Load an ELF binary into a specific node.
- **start_node** / **stop_node**: Control the execution lifecycle.
- **read_cpu_state**: Dump the current CPU registers (useful when a crash occurs).
- **read_memory**: Inspect raw memory addresses (e.g., to check the stack pointer or peripheral MMIO registers).
- **disassemble**: See what code is currently executing.
- **send_uart_input**: Send keyboard input to the virtual serial console.

## Available Resources

The AI can also read real-time context:

- **virtmcu://simulation/status**: A JSON object showing which nodes are currently running.
- **virtmcu://nodes/{node_id}/console**: The real-time UART output stream for a node.

## Example Scenario: Diagnosing a Crash

Imagine you have a firmware that crashes on boot. Instead of manually attaching GDB, you can ask your AI:

> "Provision a test-node with `my_board.yaml`, flash `crash.elf`, and tell me where it crashed."

The AI will autonomously:
1. Call `provision_board` and `flash_firmware`.
2. Call `start_node`.
3. Wait a moment, then read `virtmcu://nodes/node0/console` to see if an exception was printed.
4. Call `read_cpu_state` to see the PC and SP.
5. Call `disassemble` to see the exact instruction that caused the fault.
6. Explain the root cause to you in plain English!
