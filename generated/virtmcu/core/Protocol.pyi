from __future__ import annotations

import flatbuffers
import numpy as np

import typing
from typing import cast

uoffset: typing.TypeAlias = flatbuffers.number_types.UOffsetTFlags.py_type

class Protocol(object):
  Ethernet = cast(int, ...)
  Uart = cast(int, ...)
  Spi = cast(int, ...)
  CanFd = cast(int, ...)
  FlexRay = cast(int, ...)
  Lin = cast(int, ...)
  Rf802154 = cast(int, ...)
  RfHci = cast(int, ...)
  Control = cast(int, ...)

