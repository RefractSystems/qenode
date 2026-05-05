# Lesson 17: Securing the Digital Twin Boundary

## Introduction
In cyber-physical systems and digital twins, the boundary between the simulated world (virtmcu/QEMU) and the external network (Zenoh/MuJoCo) is a critical attack surface. A malformed packet from a malicious actor could theoretically crash the simulation or, worse, execute arbitrary code within the QEMU host process.

This lesson explores how virtmcu leverages Rust and fuzzing to protect this boundary.

## The Oxidization Strategy
Originally, virtmcu plugins like `netdev` were written in C. C is notoriously prone to buffer overflows, especially when parsing untrusted binary network data.

By migrating these plugins to Rust (Rust Porting), we eliminated entire classes of memory safety vulnerabilities. The Rust compiler guarantees that we cannot accidentally read past the end of a Zenoh packet array or dereference a null pointer.

## Fuzzing the Parsers
To gain empirical confidence, we employ fuzzing. Fuzzing involves feeding randomly mutated, malformed data to our parsers to see if they crash.

### Python Parsers
We use `hypothesis` to fuzz our YAML and REPL configuration parsers.
Run the tests:
```bash
pytest tests/test_parser_fuzzing.py
```

### Rust Packet Parsers
We use `proptest` to throw random byte arrays at our native Rust network header deserialization logic.
Run the tests:
```bash
cargo test --manifest-path hw/rust/common/virtmcu-api/Cargo.toml --test fuzz_network
```

## Conclusion
By combining Rust's memory safety with rigorous fuzzing, we ensure that virtmcu can safely ingest data from untrusted networks, making it suitable for large-scale, multi-tenant cyber-physical simulations.
