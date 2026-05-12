# ==============================================================================
# Makefile (Lesson 3: yaml2qemu)
#
# This Makefile triggers the translation of a .yaml file into a Device Tree
# Blob (.dtb) and provides a clean target.
# ==============================================================================

YAML_FILE = src/test_board.yaml
OUT_DTB = test_board.dtb

WORKSPACE_DIR ?= $(abspath ../../..)
VIRTMCU_CLI ?= cargo +nightly run --manifest-path $(WORKSPACE_DIR)/Cargo.toml -p virtmcu-cli --release --

# Default target: perform the translation
build:
	$(VIRTMCU_CLI) platform generate $(YAML_FILE) --out-dtb $(OUT_DTB)

clean:
	rm -f $(OUT_DTB) $(OUT_DTB).dts
