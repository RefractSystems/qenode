pub mod barrier;
pub mod message_log;
pub mod topics;
pub mod topology;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_topic_generation() {
        assert_eq!(topics::singleton::COORD_ALIVE, "sim/coord/alive");
        assert_eq!(topics::wildcard::COORD_DONE_WILDCARD, "sim/coord/*/done");
        assert_eq!(topics::templates::clock_advance("0"), "sim/clock/advance/0");
        assert_eq!(
            topics::templates::uart_port_tx("1", "2"),
            "virtmcu/uart/1/2/tx"
        );
    }
}
