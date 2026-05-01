"""
SOTA Test Module: test_analyze_coverage

Context:
This module implements tests for the test_analyze_coverage subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_analyze_coverage.
"""

import sys
from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest

# Import functions from analyze_coverage
sys.path.insert(0, str(Path(__file__).resolve().parent / ".."))
from tools.analyze_coverage import get_elf_symbols, main, parse_drcov


def test_parse_drcov_not_found(caplog):
    bbs = parse_drcov("nonexistent_file.drcov")
    assert bbs == []

    assert "Error: Coverage file nonexistent_file.drcov not found" in caplog.text


def test_parse_drcov_no_bb_table(tmp_path, caplog):
    f = tmp_path / "bad.drcov"
    f.write_bytes(b"some bad data without marker\n")
    bbs = parse_drcov(str(f))
    assert bbs == []

    assert "Error: Could not find BB Table in drcov file" in caplog.text


def test_parse_drcov_bad_count(tmp_path, caplog):
    f = tmp_path / "bad.drcov"
    f.write_bytes(b"BB Table: NaN\n")
    bbs = parse_drcov(str(f))
    assert bbs == []

    assert "Error: Could not parse BB count: NaN" in caplog.text


def test_parse_drcov_valid(tmp_path):
    f = tmp_path / "good.drcov"
    # Create valid drcov data: "BB Table: 2\n" + 2 entries of 8 bytes

    content = b"BB Table: 2\n"
    # bb_entry_t: uint32 start, uint16 size, uint16 mod_id
    content += (0x1000).to_bytes(4, "little") + (16).to_bytes(2, "little") + (0).to_bytes(2, "little")
    content += (0x1010).to_bytes(4, "little") + (8).to_bytes(2, "little") + (0).to_bytes(2, "little")
    f.write_bytes(content)

    bbs = parse_drcov(str(f))
    assert len(bbs) == 2
    assert bbs[0] == (0x1000, 0x1010)
    assert bbs[1] == (0x1010, 0x1018)


def test_get_elf_symbols_not_found(caplog):
    syms = get_elf_symbols("nonexistent_file.elf")
    assert syms == []

    assert "Error: ELF file nonexistent_file.elf not found" in caplog.text


@patch("tools.analyze_coverage.ELFFile")
def test_get_elf_symbols_valid(mock_elf_file_cls, tmp_path):
    f = tmp_path / "dummy.elf"
    f.touch()

    # Mock ELF structures
    mock_elf = MagicMock()
    mock_elf_file_cls.return_value = mock_elf

    import elftools.elf.sections

    mock_section = MagicMock(spec=elftools.elf.sections.SymbolTableSection)

    sym1 = MagicMock()
    sym1.name = "func1"
    sym1.__getitem__.side_effect = lambda k: {"st_info": {"type": "STT_FUNC"}, "st_value": 0x1000, "st_size": 16}[k]

    sym2 = MagicMock()
    sym2.name = "func2"
    sym2.__getitem__.side_effect = lambda k: {"st_info": {"type": "STT_FUNC"}, "st_value": 0x1020, "st_size": 0}[k]

    sym3 = MagicMock()
    sym3.name = "func3"
    sym3.__getitem__.side_effect = lambda k: {"st_info": {"type": "STT_FUNC"}, "st_value": 0x1030, "st_size": 0}[k]

    mock_section.iter_symbols.return_value = [sym1, sym2, sym3]
    mock_elf.iter_sections.return_value = [mock_section]

    syms = get_elf_symbols(str(f))
    assert len(syms) == 3
    assert syms[0]["name"] == "func1"
    assert syms[0]["size"] == 16
    assert syms[1]["name"] == "func2"
    assert syms[1]["size"] == 16  # Automatically derived from next address
    assert syms[2]["name"] == "func3"
    assert syms[2]["size"] == 16  # Fallback since it's the last one


@patch("sys.argv", ["analyze_coverage.py", "dummy.drcov", "dummy.elf"])
@patch("tools.analyze_coverage.parse_drcov")
@patch("tools.analyze_coverage.get_elf_symbols")
def test_main_no_bbs(mock_get_elf_symbols, mock_parse_drcov, caplog):  # noqa: ARG001
    mock_parse_drcov.return_value = []
    with pytest.raises(SystemExit) as e:
        main()
    assert e.value.code == 1

    assert "No execution data found." in caplog.text


@patch("sys.argv", ["analyze_coverage.py", "dummy.drcov", "dummy.elf"])
@patch("tools.analyze_coverage.parse_drcov")
@patch("tools.analyze_coverage.get_elf_symbols")
def test_main_no_symbols(mock_get_elf_symbols, mock_parse_drcov, caplog):
    mock_parse_drcov.return_value = [(0x1000, 16)]
    mock_get_elf_symbols.return_value = []
    with pytest.raises(SystemExit) as e:
        main()
    assert e.value.code == 1

    assert "No symbols found to analyze." in caplog.text


@patch("sys.argv", ["analyze_coverage.py", "dummy.drcov", "dummy.elf", "--fail-under", "100"])
@patch("tools.analyze_coverage.parse_drcov")
@patch("tools.analyze_coverage.get_elf_symbols")
def test_main_coverage_success_and_failure(mock_get_elf_symbols, mock_parse_drcov, caplog):
    mock_parse_drcov.return_value = [(0x1000, 0x1008)]  # Only half of func1 covered

    mock_get_elf_symbols.return_value = [{"name": "func1", "address": 0x1000, "size": 16}]

    with pytest.raises(SystemExit) as e:
        main()
    assert e.value.code == 1  # Should fail because fail-under=100 and we have 50%

    assert "func1                          Yes            50.0%" in caplog.text
    assert "FAILED: Coverage 50.0% is below required 100.0%" in caplog.text


@patch("sys.argv", ["analyze_coverage.py", "dummy.drcov", "dummy.elf"])
@patch("tools.analyze_coverage.parse_drcov")
@patch("tools.analyze_coverage.get_elf_symbols")
def test_main_coverage_pass(mock_get_elf_symbols, mock_parse_drcov, caplog):
    mock_parse_drcov.return_value = [(0x1000, 0x1010)]
    mock_get_elf_symbols.return_value = [{"name": "func1", "address": 0x1000, "size": 16}]

    main()  # Shouldn't exit

    assert "func1                          Yes           100.0%" in caplog.text
    assert "Coverage check passed." in caplog.text
