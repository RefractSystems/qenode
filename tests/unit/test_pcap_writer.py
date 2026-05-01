"""
SOTA Test Module: test_pcap_writer

Context:
This module implements tests for the test_pcap_writer subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_pcap_writer.
"""

from pathlib import Path

from tools.testing.virtmcu_test_suite.pcap_writer import write_pcap


def test_write_pcap_empty(tmp_path: Path):
    out_file = tmp_path / "test.pcap"
    write_pcap(out_file, [])
    assert out_file.exists()
    content = out_file.read_bytes()
    assert len(content) == 24
    assert content[:4] == (0xA1B2C3D4).to_bytes(4, "little")


def test_write_pcap_valid_entries(tmp_path: Path):
    out_file = tmp_path / "test.pcap"
    history = [
        {"vtime_ns": 1_000_000, "topic": "test/1", "payload": "deadbeef", "direction": "tx"},
        {"vtime_ns": 2_500_000, "topic": "test/2", "payload": "", "direction": "rx"},
    ]
    write_pcap(out_file, history)
    content = out_file.read_bytes()
    assert len(content) > 24

    # Check that lengths are correctly bounded
    pcap_payload = (
        (0).to_bytes(4, "little")
        + (0).to_bytes(4, "little")
        + (255).to_bytes(2, "little")
        + (6).to_bytes(2, "little")
        + b"test/1"
        + (2).to_bytes(2, "little")
        + b"tx"
        + b"\xde\xad\xbe\xef"
    )

    # Length of first payload
    orig_len = len(pcap_payload)
    # The record should contain the 16 byte packet header + the payload
    assert content[24 + 12 : 24 + 16] == orig_len.to_bytes(4, "little")
    assert content[24 + 16 : 24 + 16 + orig_len] == pcap_payload


def test_write_pcap_malformed_entries(tmp_path: Path):
    out_file = tmp_path / "test.pcap"
    history = [
        {"vtime_ns": None, "topic": None, "payload": "not hex!", "direction": None},  # Should handle gracefully
    ]
    write_pcap(out_file, history)
    content = out_file.read_bytes()
    assert len(content) > 24


def test_write_pcap_truncation(tmp_path: Path):
    out_file = tmp_path / "test.pcap"
    long_hex = "00" * 70000  # 70000 bytes > 65535 snaplen
    history = [{"vtime_ns": 0, "topic": "big", "payload": long_hex, "direction": "tx"}]
    write_pcap(out_file, history)
    content = out_file.read_bytes()

    # 24 byte header + 16 byte packet header + 65535 truncated payload
    assert len(content) == 24 + 16 + 65535
