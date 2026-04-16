#[repr(C, packed)]
#[derive(Debug, Copy, Clone)]
pub struct ClockAdvanceReq {
    pub delta_ns: u64,
    pub mujoco_time_ns: u64,
}

#[repr(C, packed)]
#[derive(Debug, Copy, Clone)]
pub struct ClockReadyResp {
    pub current_vtime_ns: u64,
    pub n_frames: u32,
    pub error_code: u32, // 0=OK, 1=STALL
}
