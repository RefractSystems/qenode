/// Topics module for standard Zenoh routing
pub mod sim_topic {
    use alloc::format;
    use alloc::string::String;

    /// Generates the RX topic for a given character device.
    pub fn chardev_rx(node_id: &str) -> String {
        format!("sim/chardev/{node_id}/rx")
    }
    /// Generates the TX topic for a given character device.
    pub fn chardev_tx(node_id: &str) -> String {
        format!("sim/chardev/{node_id}/tx")
    }

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
}

#[cfg(test)]
mod tests {
    use super::sim_topic::*;
    use alloc::format;

    #[test]
    fn test_chardev_rx_topic_node0() {
        assert_eq!(chardev_rx("0"), "sim/chardev/0/rx");
    }

    #[test]
    fn test_chardev_tx_topic_node0() {
        assert_eq!(chardev_tx("0"), "sim/chardev/0/tx");
    }

    #[test]
    fn test_chardev_rx_tx_distinct() {
        assert_ne!(chardev_rx("0"), chardev_tx("0"));
    }

    #[test]
    fn test_chardev_multi_node_isolation() {
        assert_ne!(chardev_rx("0"), chardev_rx("1"));
    }

    #[test]
    fn test_clock_advance_topic() {
        assert_eq!(clock_advance("0"), "sim/clock/advance/0");
        assert_eq!(clock_advance("3"), "sim/clock/advance/3");
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
        assert_eq!(coord_tx("2"), "sim/coord/2/tx");
        assert_eq!(coord_rx("2"), "sim/coord/2/rx");
        assert_eq!(coord_done("2"), "sim/coord/2/done");
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
        let topics =
            [chardev_rx("0"), chardev_tx("0"), clock_advance("0"), coord_tx("0"), eth_rx("0")];
        for topic in topics {
            assert!(!topic.ends_with('/'), "topic has trailing slash: {}", topic);
        }
    }

    #[test]
    fn test_node_id_int_or_str_accepted() {
        for node in ["0", "1", "15", "255"] {
            let topic = chardev_rx(node);
            assert!(topic.contains(&format!("/{}/", node)));
        }
        assert_eq!(chardev_rx("alpha"), "sim/chardev/alpha/rx");
    }
}
