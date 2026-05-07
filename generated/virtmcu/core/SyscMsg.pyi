from __future__ import annotations

import flatbuffers
import numpy as np

import typing

uoffset: typing.TypeAlias = flatbuffers.number_types.UOffsetTFlags.py_type

class SyscMsg(object):
  @classmethod
  def SizeOf(cls) -> int: ...

  def Init(self, buf: bytes, pos: int) -> None: ...
  def Type_(self) -> int: ...
  def IrqNum(self) -> int: ...
  def Data(self) -> int: ...

def CreateSyscMsg(builder: flatbuffers.Builder, type_: int, irqNum: int, data: int) -> uoffset: ...

