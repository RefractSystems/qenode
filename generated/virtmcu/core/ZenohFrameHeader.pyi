from __future__ import annotations

import flatbuffers
import numpy as np

import typing

uoffset: typing.TypeAlias = flatbuffers.number_types.UOffsetTFlags.py_type

class ZenohFrameHeader(object):
  @classmethod
  def SizeOf(cls) -> int: ...

  def Init(self, buf: bytes, pos: int) -> None: ...
  def DeliveryVtimeNs(self) -> int: ...
  def SequenceNumber(self) -> int: ...
  def Size(self) -> int: ...

def CreateZenohFrameHeader(builder: flatbuffers.Builder, deliveryVtimeNs: int, sequenceNumber: int, size: int) -> uoffset: ...

