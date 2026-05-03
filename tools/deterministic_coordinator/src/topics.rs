// AUTO-GENERATED from topics.toml. DO NOT EDIT MANUALLY.
#![allow(dead_code)]

pub mod singleton {
    pub const COORD_ALIVE: &str = "sim/coord/alive";
    pub const ROUTER_CHECK: &str = "sim/router/check";
    pub const NETWORK_CONTROL: &str = "sim/network/control";
}

pub mod wildcard {
    pub const COORD_DONE_WILDCARD: &str = "sim/coord/*/done";
    pub const COORD_RX_WILDCARD: &str = "sim/coord/*/rx";
    pub const COORD_TX_WILDCARD: &str = "sim/coord/*/tx";
    pub const ETH_FRAME_RX_WILDCARD: &str = "sim/eth/frame/*/rx";
    pub const ETH_FRAME_TX_WILDCARD: &str = "sim/eth/frame/*/tx";
    pub const SIM_UART_TX_WILDCARD: &str = "sim/uart/*/tx";
    pub const SIM_UART_RX_WILDCARD: &str = "sim/uart/*/rx";
    pub const CAN_TX_WILDCARD: &str = "sim/can/*/tx";
    pub const LIN_TX_WILDCARD: &str = "sim/lin/*/tx";
    pub const SIM_SYSTEMC_TX_WILDCARD: &str = "sim/systemc/frame/*/tx";
    pub const SPI_TX_WILDCARD: &str = "sim/spi/*/*/tx";
    pub const RF_IEEE802154_TX_WILDCARD: &str = "sim/rf/ieee802154/*/tx";
    pub const RF_HCI_TX_WILDCARD: &str = "sim/rf/hci/*/tx";
    pub const CHARDEV_TX_WILDCARD: &str = "sim/chardev/*/tx";
    pub const NETDEV_TX_WILDCARD: &str = "sim/netdev/*/tx";
    pub const VIRTM_UART_TX_WILDCARD: &str = "virtmcu/uart/*/tx";
    pub const VIRTM_UART_PORT_TX_WILDCARD: &str = "virtmcu/uart/**/tx";
}

pub const ALL_LEGACY_TX_WILDCARDS: &[&str] = &[
    wildcard::ETH_FRAME_TX_WILDCARD,
    wildcard::SIM_UART_TX_WILDCARD,
    wildcard::CAN_TX_WILDCARD,
    wildcard::LIN_TX_WILDCARD,
    wildcard::SIM_SYSTEMC_TX_WILDCARD,
    wildcard::SPI_TX_WILDCARD,
    wildcard::RF_IEEE802154_TX_WILDCARD,
    wildcard::RF_HCI_TX_WILDCARD,
    wildcard::CHARDEV_TX_WILDCARD,
    wildcard::NETDEV_TX_WILDCARD,
    wildcard::VIRTM_UART_TX_WILDCARD,
    wildcard::VIRTM_UART_PORT_TX_WILDCARD,
];

pub mod templates {
    pub fn clock_advance(node_id: &str) -> String {
        format!("sim/clock/advance/{}", node_id)
    }
    pub fn clock_start(node_id: &str) -> String {
        format!("sim/clock/start/{}", node_id)
    }
    pub fn clock_vtime(node_id: &str) -> String {
        format!("sim/clock/vtime/{}", node_id)
    }
    pub fn clock_liveliness(node_id: &str) -> String {
        format!("sim/clock/liveliness/{}", node_id)
    }
    pub fn clock_heartbeat(node_id: &str) -> String {
        format!("sim/clock/heartbeat/{}", node_id)
    }
    pub fn clock_unique_prefix(unique_id: &str) -> String {
        format!("sim/clock/{}", unique_id)
    }
    pub fn coord_done(node_id: &str) -> String {
        format!("sim/coord/{}/done", node_id)
    }
    pub fn coord_rx(node_id: &str) -> String {
        format!("sim/coord/{}/rx", node_id)
    }
    pub fn coord_tx(node_id: &str) -> String {
        format!("sim/coord/{}/tx", node_id)
    }
    pub fn plugin_liveliness(plugin: &str, node_id: &str) -> String {
        format!("sim/{}/liveliness/{}", plugin, node_id)
    }
    pub fn test_probe(suffix: &str) -> String {
        format!("sim/test/probe/{}", suffix)
    }
    pub fn eth_tx(node_id: &str) -> String {
        format!("sim/eth/frame/{}/tx", node_id)
    }
    pub fn eth_rx(node_id: &str) -> String {
        format!("sim/eth/frame/{}/rx", node_id)
    }
    pub fn uart_tx(node_id: &str) -> String {
        format!("virtmcu/uart/{}/tx", node_id)
    }
    pub fn uart_rx(node_id: &str) -> String {
        format!("virtmcu/uart/{}/rx", node_id)
    }
    pub fn uart_unique_prefix(unique_id: &str) -> String {
        format!("virtmcu/uart/{}", unique_id)
    }
    pub fn sim_uart_tx(node_id: &str) -> String {
        format!("sim/uart/{}/tx", node_id)
    }
    pub fn sim_uart_rx(node_id: &str) -> String {
        format!("sim/uart/{}/rx", node_id)
    }
    pub fn uart_port_tx(node_id: &str, port_id: &str) -> String {
        format!("virtmcu/uart/{}/{}/tx", node_id, port_id)
    }
    pub fn uart_port_rx(node_id: &str, port_id: &str) -> String {
        format!("virtmcu/uart/{}/{}/rx", node_id, port_id)
    }
    pub fn can_tx(node_id: &str) -> String {
        format!("sim/can/{}/tx", node_id)
    }
    pub fn can_rx(node_id: &str) -> String {
        format!("sim/can/{}/rx", node_id)
    }
    pub fn lin_tx(node_id: &str) -> String {
        format!("sim/lin/{}/tx", node_id)
    }
    pub fn lin_rx(node_id: &str) -> String {
        format!("sim/lin/{}/rx", node_id)
    }
    pub fn lin_unique_prefix(unique_id: &str) -> String {
        format!("sim/lin/{}", unique_id)
    }
    pub fn flexray_unique_prefix(unique_id: &str) -> String {
        format!("sim/flexray/{}", unique_id)
    }
    pub fn spi_base(bus: &str, node_id: &str) -> String {
        format!("sim/spi/{}/{}", bus, node_id)
    }
    pub fn rf_ieee802154_tx(node_id: &str) -> String {
        format!("sim/rf/ieee802154/{}/tx", node_id)
    }
    pub fn rf_ieee802154_rx(node_id: &str) -> String {
        format!("sim/rf/ieee802154/{}/rx", node_id)
    }
    pub fn rf_hci_tx(node_id: &str) -> String {
        format!("sim/rf/hci/{}/tx", node_id)
    }
    pub fn rf_hci_rx(node_id: &str) -> String {
        format!("sim/rf/hci/{}/rx", node_id)
    }
    pub fn telemetry_trace(node_id: &str) -> String {
        format!("sim/telemetry/trace/{}", node_id)
    }
    pub fn chardev_rx(node_id: &str) -> String {
        format!("sim/chardev/{}/rx", node_id)
    }
    pub fn chardev_tx(node_id: &str) -> String {
        format!("sim/chardev/{}/tx", node_id)
    }
    pub fn netdev_rx(node_id: &str) -> String {
        format!("sim/netdev/{}/rx", node_id)
    }
    pub fn netdev_tx(node_id: &str) -> String {
        format!("sim/netdev/{}/tx", node_id)
    }
}
