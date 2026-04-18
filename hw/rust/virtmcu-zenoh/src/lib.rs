use core::ffi::c_char;
use std::ffi::CStr;
use zenoh::{Config, Session, Wait};

/// Opens a Zenoh session with a standardized config for virtmcu.
/// 
/// If `router` is provided and non-empty, it is used as a connect endpoint.
/// Scouting is disabled if a router is provided.
pub fn open_session(router: *const c_char) -> Result<Session, zenoh::Error> {
    let mut config = Config::default();
    
    if !router.is_null() {
        unsafe {
            if let Ok(r_str) = CStr::from_ptr(router).to_str() {
                if !r_str.is_empty() {
                    let json = format!("[\"{}\"]", r_str);
                    let _ = config.insert_json5("connect/endpoints", &json);
                    let _ = config.insert_json5("scouting/multicast/enabled", "false");
                }
            }
        }
    }

    zenoh::open(config).wait()
}
