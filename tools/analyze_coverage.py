#!/usr/bin/env python3
import argparse
import os
import struct
import sys

from elftools.elf.elffile import ELFFile
from elftools.elf.sections import SymbolTableSection


def parse_drcov(filename):
    if not os.path.exists(filename):
        print(f"Error: Coverage file {filename} not found")
        return []

    with open(filename, "rb") as f:
        content = f.read()

    # Simple search for "BB Table: "
    marker = b"BB Table: "
    idx = content.find(marker)
    if idx == -1:
        print("Error: Could not find BB Table in drcov file")
        return []

    # Find number of BBs
    end_idx = content.find(b"\n", idx)
    count_str = content[idx + len(marker) : end_idx].decode().strip()
    try:
        count = int(count_str.split()[0])
    except Exception:
        print(f"Error: Could not parse BB count: {count_str}")
        return []

    data = content[end_idx + 1 :]
    bbs = []
    # bb_entry_t: uint32 start, uint16 size, uint16 mod_id
    entry_size = 8
    for i in range(count):
        entry = data[i * entry_size : (i + 1) * entry_size]
        if len(entry) < entry_size:
            break
        start, size, mod_id = struct.unpack("<IHH", entry)
        bbs.append((start, size))

    return bbs


def get_elf_symbols(elf_path):
    symbols = []
    if not os.path.exists(elf_path):
        print(f"Error: ELF file {elf_path} not found")
        return []

    with open(elf_path, "rb") as f:
        elffile = ELFFile(f)
        for section in elffile.iter_sections():
            if not isinstance(section, SymbolTableSection):
                continue

            for symbol in section.iter_symbols():
                # We care about text symbols
                if symbol["st_info"]["type"] in ["STT_FUNC", "STT_NOTYPE"] and symbol["st_value"] != 0:
                    if symbol.name and not symbol.name.startswith("$"):  # Skip ARM mapping symbols
                        symbols.append({"name": symbol.name, "address": symbol["st_value"], "size": symbol["st_size"]})

    # Sort and remove duplicates or handle overlaps
    symbols = sorted(symbols, key=lambda x: x["address"])

    # Refine sizes for assembly or stripped files
    for i in range(len(symbols) - 1):
        if symbols[i]["size"] == 0:
            symbols[i]["size"] = symbols[i + 1]["address"] - symbols[i]["address"]

    # Try to find the end of the text section for the last symbol
    if symbols and symbols[-1]["size"] == 0:
        symbols[-1]["size"] = 16  # Fallback

    return symbols


def main():
    parser = argparse.ArgumentParser(description="Analyze Guest Firmware Coverage")
    parser.add_argument("drcov", help="Path to .drcov file")
    parser.add_argument("elf", help="Path to ELF firmware file")
    parser.add_argument("--fail-under", type=float, help="Fail if total coverage is below this percentage")
    parser.add_argument("--verbose", action="store_true", help="Print all functions")

    args = parser.parse_args()

    bbs = parse_drcov(args.drcov)
    if not bbs:
        print("No execution data found.")
        sys.exit(1)

    symbols = get_elf_symbols(args.elf)
    if not symbols:
        print("No symbols found to analyze.")
        sys.exit(1)

    print(f"Coverage Report for {args.elf}")
    print(f"BBs: {len(bbs)}, Functions: {len(symbols)}")
    print("-" * 60)
    print(f"{'Function Name':<30} {'Executed?':<10} {'Coverage':<10}")
    print("-" * 60)

    # Create a set of all executed addresses for O(1) lookup
    executed_addresses = set()
    for bb_start, bb_size in bbs:
        # We only care about addresses that are within the range of our symbols
        # to keep the set size reasonable.
        for addr in range(bb_start, bb_start + bb_size):
            executed_addresses.add(addr)

    total_func_size = 0
    total_exec_size = 0

    results = []
    for sym in symbols:
        name = sym["name"]
        addr = sym["address"]
        size = sym["size"]

        if size == 0:
            continue

        # Count how many bytes in this symbol range were executed
        exec_count = 0
        for i in range(addr, addr + size):
            if i in executed_addresses:
                exec_count += 1

        coverage = (exec_count / size) * 100 if size > 0 else 0
        executed = "Yes" if exec_count > 0 else "No"

        if args.verbose or executed == "Yes" or coverage < 100:
            results.append((name, executed, coverage))

        total_func_size += size
        total_exec_size += exec_count

    # Print top functions or all if verbose
    for name, exec_status, cov in results:
        print(f"{name:<30} {exec_status:<10} {cov:>8.1f}%")

    print("-" * 60)
    total_coverage = (total_exec_size / total_func_size * 100) if total_func_size > 0 else 0
    print(f"{'TOTAL':<30} {'':<10} {total_coverage:>8.1f}%")
    print("-" * 60)

    if args.fail_under and total_coverage < args.fail_under:
        print(f"FAILED: Coverage {total_coverage:.1f}% is below required {args.fail_under:.1f}%")
        sys.exit(1)

    print("Coverage check passed.")


if __name__ == "__main__":
    main()
