#!/usr/bin/env python3
import logging
import re
import sys
from pathlib import Path

logger = logging.getLogger(__name__)


def main():
    if len(sys.argv) < 2:
        logger.info("Usage: apply_rust_asan_fix.py <qemu_dir>")
        sys.exit(1)

    qemu_dir = Path(sys.argv[1])
    meson_build = qemu_dir / "meson.build"

    if not meson_build.exists():
        logger.info(f"  -> {meson_build} not found, skipping Rust ASan patch")
        sys.exit(0)

    content = meson_build.read_text()
    changed = False

    # 1. Patch AddressSanitizer (asan)
    rust_asan = "add_project_arguments('-C', 'link-arg=-fsanitize=address', language: 'rust')"
    if "get_option('b_sanitize').contains('address')" not in content:
        pattern_c = r"(\s+)(qemu_cflags\s+=\s+\['-fsanitize=address'\]\s+\+\s+qemu_cflags\n\s+qemu_ldflags\s+=\s+\['-fsanitize=address'\]\s+\+\s+qemu_ldflags)"
        match_c = re.search(pattern_c, content)
        if match_c:
            indent = match_c.group(1)
            original_c = match_c.group(2)
            new_c = f"if not get_option('b_sanitize').contains('address')\n{indent}  {original_c}\n{indent}endif"
            content = content.replace(original_c, new_c)
            changed = True
            logger.info("  -> Wrapped C ASan flags")

        pattern_rust = r"(\s+)if have_rust\s*\n\s*" + re.escape(rust_asan) + r"\s*\n\s*endif"
        match_rust = re.search(pattern_rust, content)
        if match_rust:
            indent = match_rust.group(1)
            new_rust = f"{indent}if have_rust and not get_option('b_sanitize').contains('address')\n{indent}  {rust_asan}\n{indent}endif"
            content = content.replace(match_rust.group(0), new_rust)
            changed = True
            logger.info("  -> Wrapped Rust ASan flags")
        elif rust_asan not in content:
            pattern_insert = r"(if not get_option\('b_sanitize'\)\.contains\('address'\)\n\s+qemu_cflags.*\n\s+qemu_ldflags.*\n\s+endif)"
            match_insert = re.search(pattern_insert, content)
            if match_insert:
                insertion = f"\n\n    if have_rust and not get_option('b_sanitize').contains('address')\n      {rust_asan}\n    endif"
                content = content.replace(match_insert.group(1), match_insert.group(1) + insertion)
                changed = True
                logger.info("  -> Added Rust ASan flags with check")

    # 2. Patch UndefinedBehaviorSanitizer (ubsan)
    rust_ubsan = "add_project_arguments('-C', 'link-arg=-fsanitize=undefined', language: 'rust')"
    if "get_option('b_sanitize').contains('undefined')" not in content:
        pattern_c = (
            r"(\s+)(qemu_cflags\s+\+=\s+\['-fsanitize=undefined'\]\n\s+qemu_ldflags\s+\+=\s+\['-fsanitize=undefined'\])"
        )
        match_c = re.search(pattern_c, content)
        if match_c:
            indent = match_c.group(1)
            original_c = match_c.group(2)
            new_c = f"if not get_option('b_sanitize').contains('undefined')\n{indent}  {original_c}\n{indent}endif"
            content = content.replace(original_c, new_c)
            changed = True
            logger.info("  -> Wrapped C UBSan flags")

        pattern_rust = r"(\s+)if have_rust\s*\n\s*" + re.escape(rust_ubsan) + r"\s*\n\s*endif"
        match_rust = re.search(pattern_rust, content)
        if match_rust:
            indent = match_rust.group(1)
            new_rust = f"{indent}if have_rust and not get_option('b_sanitize').contains('undefined')\n{indent}  {rust_ubsan}\n{indent}endif"
            content = content.replace(match_rust.group(0), new_rust)
            changed = True
            logger.info("  -> Wrapped Rust UBSan flags")
        elif rust_ubsan not in content:
            pattern_insert = r"(if not get_option\('b_sanitize'\)\.contains\('undefined'\)\n\s+qemu_cflags.*\n\s+qemu_ldflags.*\n\s+endif)"
            match_insert = re.search(pattern_insert, content)
            if match_insert:
                insertion = f"\n\n    if have_rust and not get_option('b_sanitize').contains('undefined')\n      {rust_ubsan}\n    endif"
                content = content.replace(match_insert.group(1), match_insert.group(1) + insertion)
                changed = True
                logger.info("  -> Added Rust UBSan flags with check")

    # 3. Patch ThreadSanitizer (tsan)
    if "get_option('b_sanitize').contains('thread')" not in content:
        pattern_c = r"(\s+)(qemu_cflags\s+=\s+\['-fsanitize=thread'\]\s+\+\s+tsan_warn_suppress\s+\+\s+qemu_cflags\n\s+qemu_ldflags\s+=\s+\['-fsanitize=thread'\]\s+\+\s+qemu_ldflags)"
        match_c = re.search(pattern_c, content)
        if match_c:
            indent = match_c.group(1)
            original_c = match_c.group(2)
            new_c = f"if not get_option('b_sanitize').contains('thread')\n{indent}  {original_c}\n{indent}endif"
            content = content.replace(original_c, new_c)
            changed = True
            logger.info("  -> Wrapped C TSan flags")

    if changed:
        meson_build.write_text(content)
        logger.info("✓ Patched meson.build for Rust ASan/UBSan/TSan support (with b_sanitize awareness)")


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
