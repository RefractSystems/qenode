use core::ffi::c_char;
use std::ffi::CStr;
use std::time::Duration;
use zenoh::{Config, Session, Wait};

/// Opens a Zenoh session with a standardized config for virtmcu.
///
/// If `router` is provided and non-empty, it is used as a connect endpoint.
/// Scouting is disabled if a router is provided.
///
/// # Safety
///
/// The caller must ensure that `router` is either NULL or a valid, null-terminated
/// C string that remains valid for the duration of this call.
pub unsafe fn open_session(router: *const c_char) -> Result<Session, zenoh::Error> {
    let mut config = Config::default();
    let mut has_router = false;

    if !router.is_null() {
        if let Ok(r_str) = CStr::from_ptr(router).to_str() {
            if !r_str.is_empty() {
                let json = format!("[\"{}\"]", r_str);
                let _ = config.insert_json5("mode", "\"client\"");
                let _ = config.insert_json5("connect/endpoints", &json);
                let _ = config.insert_json5("scouting/multicast/enabled", "false");
                let _ = config.insert_json5("transport/shared_memory/enabled", "false");
                has_router = true;
            }
        }
    }

    let session = zenoh::open(config)
        .wait()
        .map_err(|e| zenoh::Error::from(format!("Failed to open Zenoh session: {}", e)))?;
    virtmcu_qom::vlog!("[virtmcu-zenoh] Session returned from zenoh::open.wait().\n");

    // If a router was provided, verify we can actually reach it.
    // In Zenoh 1.0, open() returns successfully even if the remote endpoint is unreachable.
    // virtmcu smoke tests expect immediate failure for unreachable explicit routers.
    if has_router {
        // We check for any active connections to routers/peers.
        // We wait a bit for the connection to be established.
        let mut connected = false;
        for i in 0..40 {
            // Increased from 10 to 40 (2 seconds)
            let info = session.info();
            let routers: Vec<_> = info.routers_zid().wait().collect();
            let peers: Vec<_> = info.peers_zid().wait().collect();

            if !routers.is_empty() || !peers.is_empty() {
                virtmcu_qom::vlog!(
                    "[virtmcu-zenoh] Connected after {} attempts. Routers={:?}, Peers={:?}",
                    i,
                    routers,
                    peers
                );
                connected = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        if !connected {
            virtmcu_qom::vlog!(
                "[virtmcu-zenoh] Failed to connect to explicit router after 2 seconds."
            );
            let _ = session.close().wait();
            return Err(zenoh::Error::from(
                "Failed to connect to explicit router".to_string(),
            ));
        }
    }

    Ok(session)
}
