# ==============================================================================
# Makefile (Lesson 3: yaml2qemu)
#
# This Makefile triggers the translation of a .yaml file into a Device Tree
# Blob (.dtb) and provides a clean target.
# ==============================================================================

YAML_FILE = src/test_board.yaml
OUT_DTB = test_board.dtb

# Default target: perform the translation
build:
	python3 -m tools.yaml2qemu $(YAML_FILE) --out-dtb $(OUT_DTB)

clean:
	rm -f $(OUT_DTB) $(OUT_DTB).dts
