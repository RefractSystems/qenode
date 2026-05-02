from __future__ import annotations

from typing import Any, Literal

import yaml
from pydantic import BaseModel, ConfigDict, Field


class MachineSpec(BaseModel):
    model_config = ConfigDict(extra="allow")
    name: str | None = None
    type: str | None = None
    cpus: list[dict[str, Any]] | None = None


class NodeSpec(BaseModel):
    name: str | int


class WireLink(BaseModel):
    protocol: str = Field(alias="type")
    nodes: list[str | int]
    baud: int | None = None


class WirelessNode(BaseModel):
    name: str | int
    initial_position: list[float]


class WirelessMedium(BaseModel):
    medium: str
    nodes: list[WirelessNode]
    max_range_m: float


class TopologySpec(BaseModel):
    model_config = ConfigDict(extra="allow")
    nodes: list[NodeSpec] | None = None
    links: list[WireLink] = Field(default_factory=list)
    wireless: WirelessMedium | None = None
    global_seed: int = 0
    transport: Literal["zenoh", "unix"] = "zenoh"
    max_messages_per_node_per_quantum: int = 1024


class WorldYaml(BaseModel):
    model_config = ConfigDict(extra="allow")
    machine: MachineSpec | None = None
    topology: TopologySpec | None = None
    peripherals: list[dict[str, Any]] | None = None
    memory: list[dict[str, Any]] | None = None

    @classmethod
    def from_text(cls, s: str) -> WorldYaml:
        data = yaml.safe_load(s)
        return cls.model_validate(data)

    def to_yaml(self) -> str:
        # Use by_alias=True to ensure "type" is used for WireLink
        data = self.model_dump(exclude_none=True, by_alias=True)
        return str(yaml.dump(data, sort_keys=False))
