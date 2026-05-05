# Lesson 15 Makefile

.PHONY: all clean test

all: board.dtb

board.dtb: src/board.yaml
	@echo "==> Generating DTB using yaml2qemu..."
	# This assumes you have installed the virtmcu-tools package
	yaml2qemu src/board.yaml --out-dtb board.dtb
	@echo "✓ board.dtb generated."

clean:
	rm -f board.dtb

test: all
	@echo "==> Verifying board.dtb..."
	@if [ -f board.dtb ]; then \
		echo "✓ board.dtb exists."; \
	else \
		echo "FAILED: board.dtb not found."; \
		exit 1; \
	fi
