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
    nodes: list[NodeSpec] | None = None

    @classmethod
    def from_text(cls, s: str) -> WorldYaml:
        data = yaml.safe_load(s)
        return cls.model_validate(data)

    def model_post_init(self, __context: Any) -> None:  # noqa: ANN401
        # Task 2.2: Split-Brain Schema Rejection
        # If topology.nodes is populated AND top-level nodes is populated, raise error.
        has_topology_nodes = self.topology and self.topology.nodes
        has_toplevel_nodes = self.nodes

        if has_topology_nodes and has_toplevel_nodes:
            raise ValueError(
                "Split-brain YAML detected: both 'topology.nodes' and top-level 'nodes' are present. "
                "Please migrate to 'topology.nodes' exclusively."
            )

        # Also check for numeric peripherals which indicates legacy topology
        if self.peripherals:
            has_numeric_periphs = any(str(p.get("name", "")).isdigit() for p in self.peripherals)
            if has_topology_nodes and has_numeric_periphs:
                 raise ValueError(
                    "Split-brain YAML detected: 'topology.nodes' is present but 'peripherals' contains numeric node IDs. "
                    "In modern topology mode, node definitions belong exclusively in 'topology.nodes'."
                )

    def get_node_ids(self) -> set[str]:
        """Returns a set of all valid node IDs in this world."""
        res = set()
        
        # 1. Try topology.nodes
        if self.topology and self.topology.nodes:
            for n in self.topology.nodes:
                res.add(str(n.name))
            return res
            
        # 2. Try top-level nodes
        if self.nodes:
            for n in self.nodes:
                res.add(str(n.name))
            return res
            
        # 3. Fallback to peripherals if they look like numeric IDs
        if self.peripherals:
            fallback_res = set()
            all_numeric = True
            for p in self.peripherals:
                name = str(p.get("name", ""))
                if name.isdigit():
                    fallback_res.add(name)
                else:
                    all_numeric = False
                    break
            
            if all_numeric and fallback_res:
                return fallback_res

        return res

    def to_yaml(self) -> str:
        # Use by_alias=True to ensure "type" is used for WireLink
        data = self.model_dump(exclude_none=True, by_alias=True)
        return str(yaml.dump(data, sort_keys=False))
