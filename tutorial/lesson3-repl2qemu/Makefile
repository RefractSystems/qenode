# ==============================================================================
# Makefile (Lesson 3: repl2qemu)
#
# This Makefile triggers the translation of a .repl file into a Device Tree
# Blob (.dtb) and provides a clean target.
# ==============================================================================

REPL_FILE = src/test_board.repl
OUT_DTB = test_board.dtb

# Default target: perform the translation
build:
	python3 -m tools.repl2qemu $(REPL_FILE) --out-dtb $(OUT_DTB)

clean:
	rm -f $(OUT_DTB) $(OUT_DTB).dts
