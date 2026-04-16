import os
import sys

import flatbuffers

# Add tools/telemetry_fbs to sys.path so we can import the generated flatbuffers code
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
WORKSPACE_DIR = os.path.dirname(SCRIPT_DIR)
sys.path.append(os.path.join(WORKSPACE_DIR, "tools", "telemetry_fbs"))

from Virtmcu.Telemetry import TraceEvent as TraceEventBuilder  # noqa: E402
from Virtmcu.Telemetry.TraceEvent import TraceEvent  # noqa: E402


def test_trace_event_serialization():
    """
    Test that the generated FlatBuffers Python code for TraceEvent
    can correctly serialize and deserialize a message.
    """
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
