pub mod singleton {
    pub const COORD_ALIVE: &str = "sim/coord/alive";
    pub const NETWORK_CONTROL: &str = "sim/network/control";
}

pub mod wildcard {
    pub const COORD_DONE_WILDCARD: &str = "sim/coord/done/*";
}

pub mod templates {
    pub fn clock_start(node_id: &str) -> String { format!("sim/clock/start/{}", node_id) }
    pub fn clock_advance(node_id: &str) -> String { format!("sim/clock/advance/{}", node_id) }
    pub fn clock_heartbeat(node_id: &str) -> String { format!("sim/clock/heartbeat/{}", node_id) }
    pub fn clock_liveliness(node_id: &str) -> String { format!("sim/clock/liveliness/{}", node_id) }
    pub fn clock_vtime(node_id: &str) -> String { format!("sim/clock/vtime/{}", node_id) }
    pub fn clock_unique_prefix(node_id: &str) -> String { format!("sim/clock/unique/{}", node_id) }
    pub fn coord_done(node_id: &str) -> String { format!("sim/coord/done/{}", node_id) }
}
