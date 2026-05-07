from __future__ import annotations

import flatbuffers
import numpy as np

import typing

uoffset: typing.TypeAlias = flatbuffers.number_types.UOffsetTFlags.py_type

class VirtmcuHandshake(object):
  @classmethod
  def SizeOf(cls) -> int: ...

  def Init(self, buf: bytes, pos: int) -> None: ...
  def Magic(self) -> int: ...
  def Version(self) -> int: ...

def CreateVirtmcuHandshake(builder: flatbuffers.Builder, magic: int, version: int) -> uoffset: ...

