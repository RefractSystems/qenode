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
                let _ = config.insert_json5("connect/endpoints", &json);
                let _ = config.insert_json5("scouting/multicast/enabled", "false");
                has_router = true;
            }
        }
    }

    let session = zenoh::open(config).wait()?;

    // If a router was provided, verify we can actually reach it.
    // In Zenoh 1.0, open() returns successfully even if the remote endpoint is unreachable.
    // virtmcu smoke tests expect immediate failure for unreachable explicit routers.
    if has_router {
        // We check for any active connections to routers/peers.
        // We wait a bit for the connection to be established.
        let mut connected = false;
        for _ in 0..10 {
            let info = session.info();
            if info.routers_zid().wait().next().is_some() {
                connected = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        if !connected {
            let _ = session.close().wait();
            return Err(zenoh::Error::from(
                "Failed to connect to explicit router".to_string(),
            ));
        }
    }

    Ok(session)
}
