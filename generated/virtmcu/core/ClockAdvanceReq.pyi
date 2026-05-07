from __future__ import annotations

import flatbuffers
import numpy as np

import typing

uoffset: typing.TypeAlias = flatbuffers.number_types.UOffsetTFlags.py_type

class ClockAdvanceReq(object):
  @classmethod
  def SizeOf(cls) -> int: ...

  def Init(self, buf: bytes, pos: int) -> None: ...
  def DeltaNs(self) -> int: ...
  def AbsoluteVtimeNs(self) -> int: ...
  def QuantumNumber(self) -> int: ...

def CreateClockAdvanceReq(builder: flatbuffers.Builder, deltaNs: int, absoluteVtimeNs: int, quantumNumber: int) -> uoffset: ...

