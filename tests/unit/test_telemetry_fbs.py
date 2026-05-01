"""
SOTA Test Module: test_telemetry_fbs

Context:
This module implements tests for the test_telemetry_fbs subsystem.

Objective:
Ensure correct functionality, performance, and deterministic execution of test_telemetry_fbs.
"""

from __future__ import annotations

import flatbuffers


def test_trace_event_serialization() -> None:
    """
    Test that the generated FlatBuffers Python code for TraceEvent
    can correctly serialize and deserialize a message.
    """

    from Virtmcu.Telemetry import TraceEvent as TraceEventBuilder
    from Virtmcu.Telemetry.TraceEvent import TraceEvent

    builder = flatbuffers.Builder(1024)

    # Test values
    expected_ts = 1234567890
    expected_type = 1  # IRQ
    expected_id = (2 << 16) | 5  # slot 2, pin 5
    expected_value = 1  # HIGH

    # Build the flatbuffer
    TraceEventBuilder.Start(builder)
    TraceEventBuilder.AddTimestampNs(builder, expected_ts)
    TraceEventBuilder.AddType(builder, expected_type)
    TraceEventBuilder.AddId(builder, expected_id)
    TraceEventBuilder.AddValue(builder, expected_value)
    event = TraceEventBuilder.End(builder)

    builder.Finish(event)
    buf = builder.Output()

    # Ensure it's not empty
    assert len(buf) > 0

    # Deserialize the flatbuffer
    parsed_event = TraceEvent.GetRootAs(buf, 0)

    # Assert values match
    assert parsed_event.TimestampNs() == expected_ts
    assert parsed_event.Type() == expected_type
    assert parsed_event.Id() == expected_id
    assert parsed_event.Value() == expected_value
