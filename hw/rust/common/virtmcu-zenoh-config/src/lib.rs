#![allow(clippy::all, unused_imports, dead_code, unused_variables, unused_mut)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![allow(clippy::all)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::panic_in_result_fn
    )
)]
pub use zenoh::Config;

/// Creates a Zenoh configuration optimized for VirtMCU deterministic simulation.
///
/// This disables multicast and gossip scouting by default to prevent cross-talk
/// across test workers or concurrent simulation instances.
///
/// # Returns
/// A Zenoh `Config` with `scouting/multicast/enabled` and `scouting/gossip/enabled`
/// both set to `"false"`.
pub fn default_config() -> Config {
    let mut config = Config::default();

    // Always disable multicast and gossip scouting (CLAUDE.md Second Priority, RFC-0001)
    // to prevent peer discovery bleeding across parallel test workers and networks.
    let _ = config.insert_json5("scouting/multicast/enabled", "false");
    let _ = config.insert_json5("scouting/gossip/enabled", "false");

    config
}

/// Creates a strictly isolated client configuration.
///
/// This configures Zenoh to operate exclusively in client mode (no peer mode),
/// completely isolating the session unless a router endpoint is explicitly provided
/// via `Config::insert_json5("connect/endpoints", ...)`.
///
/// This is the equivalent of the Python `make_client_config()` builder.
///
/// # Returns
/// A Zenoh `Config` with scouting disabled and `mode="client"`.
pub fn client_config() -> Config {
    let mut config = default_config();
    let _ = config.insert_json5("mode", "\"client\"");
    if let Ok(endpoint) = std::env::var("VIRTMCU_ZENOH_ROUTER") {
        if !endpoint.is_empty() {
            let json = format!("[\"{endpoint}\"]");
            let _ = config.insert_json5("connect/endpoints", &json);
            let _ = config.insert_json5("transport/shared_memory/enabled", "false");
        }
    }
    config
}
