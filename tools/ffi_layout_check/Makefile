# Makefile for FFI Layout Checks

WORKSPACE_ROOT ?= ../..
QEMU_SRC ?= $(WORKSPACE_ROOT)/third_party/qemu
QEMU_BUILD ?= $(QEMU_SRC)/build-virtmcu

CFLAGS = -I$(QEMU_SRC)/include -I$(QEMU_SRC) -I$(WORKSPACE_ROOT) -I$(QEMU_BUILD) $(shell pkg-config --cflags glib-2.0)

.PHONY: all clean run-c run-rust

all: run-c run-rust

print_sizes_c: print_sizes.c
	@echo "Compiling C checker..."
	$(CC) $(CFLAGS) $< -o $@

print_sizes_rust: print_rust_sizes.rs
	@echo "Compiling Rust checker..."
	rustc --edition 2021 -o $@ $<

run-c: print_sizes_c
	@echo "--- C Struct Sizes ---"
	@./print_sizes_c
	@echo ""

run-rust: print_sizes_rust
	@echo "--- Rust Struct Sizes ---"
	@./print_sizes_rust
	@echo ""

clean:
	rm -f print_sizes_c print_sizes_rust
