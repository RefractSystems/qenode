"""
SOTA Test Module: test_parser_fuzzing

Context:
This module implements tests for the test_parser_fuzzing subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_parser_fuzzing.
"""

import tempfile
from pathlib import Path

import lark
import yaml
from hypothesis import given
from hypothesis import strategies as st

from tools.repl2qemu.parser import parse_repl
from tools.yaml2qemu import parse_yaml_platform


@given(st.text())
def test_fuzz_yaml_parser(fuzz_data):
    try:
        with tempfile.NamedTemporaryFile(mode="w", delete=False) as f:
            f.write(fuzz_data)
            tmp_path = f.name

        try:
            parse_yaml_platform(tmp_path)
        finally:
            Path(tmp_path).unlink()

    except (yaml.YAMLError, ValueError, TypeError, KeyError, AttributeError):
        pass
    except Exception as e:
        if isinstance(e, SystemExit):
            pass  # Graceful exit from argparse/validation
        else:
            raise e


@given(st.text())
def test_fuzz_repl_parser(fuzz_data):
    try:
        parse_repl(fuzz_data)
    except (lark.exceptions.LarkError, ValueError, TypeError, KeyError, AttributeError):
        pass
    except Exception as e:
        if isinstance(e, SystemExit):
            pass
        else:
            raise e
