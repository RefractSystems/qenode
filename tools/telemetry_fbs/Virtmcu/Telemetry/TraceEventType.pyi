from __future__ import annotations

from typing import cast

import flatbuffers

type uoffset = flatbuffers.number_types.UOffsetTFlags.py_type

class TraceEventType:
  CPU_STATE = cast(int, ...)
  IRQ = cast(int, ...)
  PERIPHERAL = cast(int, ...)

