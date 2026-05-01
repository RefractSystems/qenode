from __future__ import annotations

from typing import cast

import flatbuffers

type uoffset = flatbuffers.number_types.UOffsetTFlags.py_type

class LinMessageType:
  Data = cast(int, ...)
  Break = cast(int, ...)
  Sync = cast(int, ...)

