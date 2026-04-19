import re

with open("hw/rust/zenoh-clock/src/lib.rs", "r") as f:
    text = f.read()

text = text.replace("use zenoh::Wait;", "use zenoh::Wait;\nuse virtmcu_api::{ClockAdvanceReq, ClockReadyResp};")

pattern_query = r"""    let delta = u64::from_le_bytes\(payload_bytes\[0..8\]\.try_into\(\)\.unwrap\(\)\);
    let mujoco = u64::from_le_bytes\(payload_bytes\[8..16\]\.try_into\(\)\.unwrap\(\)\);"""

replacement_query = """    let req = unsafe { std::ptr::read_unaligned(payload_bytes.as_ptr() as *const ClockAdvanceReq) };
    let delta = req.delta_ns;
    let mujoco = req.mujoco_time_ns;"""
text = re.sub(pattern_query, replacement_query, text)

pattern_resp = r"""    struct ClockReadyResp \{
        vtime_ns: u64,
        n_frames: u32,
        error_code: u32,
    \}

    let resp = ClockReadyResp \{
        vtime_ns: reached_vtime,
        n_frames: 0,
        error_code: 0,
    \};

    let mut resp_bytes = \[0u8; 16\];
    unsafe \{
        ptr::copy_nonoverlapping\(
            &resp as \*const ClockReadyResp as \*const u8,
            resp_bytes\.as_mut_ptr\(\),
            16,
        \);
    \}"""

replacement_resp = """    let resp = ClockReadyResp {
        current_vtime_ns: reached_vtime,
        n_frames: 0,
        error_code: 0,
    };

    let mut resp_bytes = [0u8; 16];
    unsafe {
        ptr::copy_nonoverlapping(
            &resp as *const ClockReadyResp as *const u8,
            resp_bytes.as_mut_ptr(),
            16,
        );
    }"""
text = re.sub(pattern_resp, replacement_resp, text)

with open("hw/rust/zenoh-clock/src/lib.rs", "w") as f:
    f.write(text)
