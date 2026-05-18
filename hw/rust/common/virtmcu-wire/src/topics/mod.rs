/// Topics module for standard Zenoh routing
pub mod sim_topic {
    use alloc::format;
    use alloc::string::String;

    /// Generates the clock advance topic for a node.
    pub fn clock_advance(node_id: &str) -> String {
        format!("sim/clock/advance/{node_id}")
    }
    /// Generates the clock heartbeat topic for a node.
    pub fn clock_heartbeat(node_id: &str) -> String {
        format!("sim/clock/heartbeat/{node_id}")
    }
    /// Generates the clock liveliness topic for a node.
    pub fn clock_liveliness(node_id: &str) -> String {
        format!("sim/clock/liveliness/{node_id}")
    }
    /// Generates the clock start topic for a node.
    pub fn clock_start(node_id: &str) -> String {
        format!("sim/clock/start/{node_id}")
    }
    /// Generates the virtual time topic for a node.
    pub fn clock_vtime(node_id: &str) -> String {
        format!("sim/clock/vtime/{node_id}")
    }

    /// Generates the RX topic for a network device.
    pub fn netdev_rx(node_id: &str) -> String {
        format!("sim/netdev/{node_id}/rx")
    }

    /// Topic for coordinator liveliness token.
    pub const COORD_ALIVE: &str = "sim/coord/alive";
    /// Topic for zenoh router check token.
    pub const ROUTER_CHECK: &str = "sim/router/check";
    /// Topic for network control events.
    pub const NETWORK_CONTROL: &str = "sim/network/control";

    /// Generates the coordinator TX topic for a node.
    pub fn coord_tx(node_id: &str) -> String {
        format!("sim/coord/{node_id}/tx")
    }
    /// Generates the coordinator RX topic for a node.
    pub fn coord_rx(node_id: &str) -> String {
        format!("sim/coord/{node_id}/rx")
    }
    /// Generates the coordinator DONE topic for a node.
    pub fn coord_done(node_id: &str) -> String {
        format!("sim/coord/{node_id}/done")
    }

    /// Generates the ethernet TX topic for a node.
    pub fn eth_tx(node_id: &str) -> String {
        format!("sim/eth/frame/{node_id}/tx")
    }
    /// Generates the ethernet RX topic for a node.
    pub fn eth_rx(node_id: &str) -> String {
        format!("sim/eth/frame/{node_id}/rx")
    }

    /// Generates the UART TX topic for a node.
    pub fn uart_tx(node_id: &str) -> String {
        format!("virtmcu/uart/{node_id}/tx")
    }
    /// Generates the UART RX topic for a node.
    pub fn uart_rx(node_id: &str) -> String {
        format!("virtmcu/uart/{node_id}/rx")
    }
    /// Generates the simulated UART TX topic for a node.
    pub fn sim_uart_tx(node_id: &str) -> String {
        format!("sim/uart/{node_id}/tx")
    }
    /// Generates the simulated UART RX topic for a node.
    pub fn sim_uart_rx(node_id: &str) -> String {
        format!("sim/uart/{node_id}/rx")
    }

    /// Wildcard topic for all coordinator DONE messages.
    pub const COORD_DONE_WILDCARD: &str = "sim/coord/*/done";
    /// Wildcard topic for all coordinator TX messages.
    pub const COORD_TX_WILDCARD: &str = "sim/coord/*/tx";
    /// Wildcard topic for all ethernet TX messages.
    pub const ETH_FRAME_TX_WILDCARD: &str = "sim/eth/frame/*/tx";

    /// Topic on which the Physical Node publishes physics triggers.
    pub const PHYSICS_TRIGGER: &str = "sim/physics/trigger";
    /// Topic on which the Physics Gateway publishes done signals.
    pub const PHYSICS_DONE: &str = "sim/physics/done";

    /// Wildcard for all telemetry position messages.
    pub const TELEMETRY_POSITION_WILDCARD: &str = "**/sim/telemetry/position";
    /// Wildcard for all telemetry trace messages.
    pub const TELEMETRY_TRACE_WILDCARD: &str = "**/sim/telemetry/trace/**";

    /// Generates the instruction trace topic for a node.
    pub fn telemetry_insn(node_id: &str) -> String {
        format!("sim/telemetry/trace/{node_id}/insn")
    }
    /// Generates the telemetry events topic for a node.
    pub fn telemetry_events(node_id: &str) -> String {
        format!("sim/telemetry/trace/{node_id}")
    }

    /// Topic prefix for actuator command output from firmware.
    /// Full topic: `firmware/control/{node_id}/{actuator_id}`
    pub fn actuator_control(node_id: &str, actuator_id: u32) -> String {
        format!("firmware/control/{node_id}/{actuator_id}")
    }
    /// Wildcard for subscribing to all actuator channels for a node.
    pub fn actuator_control_wildcard(node_id: &str) -> String {
        format!("firmware/control/{node_id}/**")
    }
    /// Wildcard for subscribing to all sensor data for a node.
    pub fn sensor_data_wildcard(node_id: &str) -> String {
        format!("sim/sensor/{node_id}/**")
    }
    /// Topic for sensor liveliness.
    pub fn sensor_liveliness(node_id: &str) -> String {
        format!("sim/sensor/liveliness/{node_id}")
    }
    /// Topic for actuator liveliness.
    pub fn actuator_liveliness(node_id: &str) -> String {
        format!("sim/actuator/liveliness/{node_id}")
    }
    /// Topic for sensor data injected into firmware.
    /// Full topic: `sim/sensor/{node_id}/sensordata_{sensor_id}`
    pub fn sensor_data(node_id: &str, sensor_id: u32) -> String {
        format!("sim/sensor/{node_id}/sensordata_{sensor_id}")
    }
}

#[cfg(test)]
mod tests {
    use super::sim_topic::*;
    use alloc::format;

    const NODE_ID_2: &str = "2";
    const NODE_ID_3: &str = "3";
    const NODE_15: &str = "15";
    const NODE_255: &str = "255";

    #[test]
    fn test_clock_advance_topic() {
        assert_eq!(clock_advance("0"), "sim/clock/advance/0");
        assert_eq!(clock_advance(NODE_ID_3), format!("sim/clock/advance/{}", NODE_ID_3));
    }

    #[test]
    fn test_clock_heartbeat_topic() {
        assert_eq!(clock_heartbeat("0"), "sim/clock/heartbeat/0");
    }

    #[test]
    fn test_clock_liveliness_topic() {
        assert_eq!(clock_liveliness("0"), "sim/clock/liveliness/0");
    }

    #[test]
    fn test_clock_vtime_topic() {
        assert_eq!(clock_vtime("0"), "sim/clock/vtime/0");
    }

    #[test]
    fn test_netdev_rx_topic() {
        assert_eq!(netdev_rx("0"), "sim/netdev/0/rx");
    }

    #[test]
    fn test_coord_alive_singleton() {
        assert_eq!(COORD_ALIVE, "sim/coord/alive");
    }

    #[test]
    fn test_router_check_singleton() {
        assert_eq!(ROUTER_CHECK, "sim/router/check");
    }

    #[test]
    fn test_network_control_singleton() {
        assert_eq!(NETWORK_CONTROL, "sim/network/control");
    }

    #[test]
    fn test_coord_per_node_topics() {
        assert_eq!(coord_tx(NODE_ID_2), format!("sim/coord/{}/tx", NODE_ID_2));
        assert_eq!(coord_rx(NODE_ID_2), format!("sim/coord/{}/rx", NODE_ID_2));
        assert_eq!(coord_done(NODE_ID_2), format!("sim/coord/{}/done", NODE_ID_2));
    }

    #[test]
    fn test_eth_topics() {
        assert_eq!(eth_tx("0"), "sim/eth/frame/0/tx");
        assert_eq!(eth_rx("1"), "sim/eth/frame/1/rx");
    }

    #[test]
    fn test_uart_namespace_split() {
        assert_eq!(uart_tx("0"), "virtmcu/uart/0/tx");
        assert_eq!(uart_rx("0"), "virtmcu/uart/0/rx");
        assert_eq!(sim_uart_tx("0"), "sim/uart/0/tx");
        assert_eq!(sim_uart_rx("0"), "sim/uart/0/rx");
    }

    #[test]
    fn test_wildcard_subscribers() {
        assert_eq!(COORD_DONE_WILDCARD, "sim/coord/*/done");
        assert_eq!(COORD_TX_WILDCARD, "sim/coord/*/tx");
        assert_eq!(ETH_FRAME_TX_WILDCARD, "sim/eth/frame/*/tx");
    }

    #[test]
    fn test_topic_no_trailing_slash() {
        let topics = [clock_advance("0"), coord_tx("0"), eth_rx("0")];
        for topic in topics {
            assert!(!topic.ends_with('/'), "topic has trailing slash: {}", topic);
        }
    }
}
