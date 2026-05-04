import "./protocols.tsp";
import "@typespec/json-schema";

using TypeSpec.JsonSchema;

namespace VirtMcu.World;

@extension("oneOf", #[
  #{ type: "integer", minimum: 0 },
  #{ type: "string" }
])
scalar NodeID extends string;

@extension("oneOf", #[
  #{ type: "integer", minimum: 0 },
  #{ type: "string", pattern: "^(0x[0-9a-fA-F]+|none|sysbus)$" }
])
scalar Address extends string;

model Coordinate {
  x: float64;
  y: float64;
  z: float64;
}

model Cpu {
  name: string;
  type: string;
  memory?: string;
  isa?: string;
  @encodedName("application/json", "mmu-type")
  mmu_type?: string;
}

model Machine {
  name?: string;
  type?: string;
  cpus?: Cpu[];
}

model Resource {
  name: NodeID;
  type?: string;
  renode_type?: string;
  address?: Address;
  size?: Address;
  parent?: string;
  container?: string;
  
  @extension("items", #{
    oneOf: #[
      #{ type: "string" },
      #{ type: "integer" }
    ]
  })
  interrupts?: string[];
  properties?: Record<unknown>;
}

model Node {
  name: NodeID;
  role?: "Cyber" | "Physics";
}

model WireLink {
  @extension("alias", "type")
  type: Protocol;
  nodes: NodeID[];
  baud?: uint32;
}

model WirelessNode {
  name: NodeID;
  initial_position: Coordinate;
}

model WirelessMedium {
  medium: string;
  max_range_m: float64;
  nodes: WirelessNode[];
}

model Topology {
  nodes?: Node[];
  links?: WireLink[];
  wireless?: WirelessMedium;
  global_seed?: uint64;
  transport?: "zenoh" | "unix";
  max_messages_per_node_per_quantum?: uint32;
}

model World {
  machine?: Machine;
  peripherals?: Resource[];
  memory?: Resource[];
  topology?: Topology;
  nodes?: Node[];
}
