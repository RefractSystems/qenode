# virtmcu-tools

Standalone tools for the **virtmcu** framework.

## Included Tools

- `repl2qemu`: Convert Renode `.repl` platform descriptions to QEMU Device Trees (`.dtb`).
- `yaml2qemu`: Convert modern virtmcu YAML platform descriptions to QEMU Device Trees.
- `virtmcu-mcp`: Model Context Protocol (MCP) server for AI-augmented debugging and orchestration.

## Installation

```bash
pip install virtmcu-tools
```

## System Dependencies

- `dtc` (Device Tree Compiler): Required for compiling `.dts` to `.dtb`.
  - Ubuntu/Debian: `sudo apt-get install device-tree-compiler`
  - macOS: `brew install dtc`
