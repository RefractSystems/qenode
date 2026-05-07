from __future__ import annotations

import flatbuffers
import numpy as np

import typing
from typing import cast

uoffset: typing.TypeAlias = flatbuffers.number_types.UOffsetTFlags.py_type

class TraceEventType(object):
  CPU_STATE = cast(int, ...)
  IRQ = cast(int, ...)
  PERIPHERAL = cast(int, ...)
  POWER_STATE = cast(int, ...)

