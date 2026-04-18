pub mod telemetry_generated;

#[repr(C, packed)]
#[derive(Debug, Copy, Clone, Default)]
pub struct ClockAdvanceReq {
    pub delta_ns: u64,
    pub mujoco_time_ns: u64,
}

#[repr(C, packed)]
#[derive(Debug, Copy, Clone, Default)]
pub struct ClockReadyResp {
    pub current_vtime_ns: u64,
    pub n_frames: u32,
    pub error_code: u32, // 0=OK, 1=STALL
}

#[repr(C, packed)]
#[derive(Debug, Copy, Clone, Default)]
pub struct ZenohFrameHeader {
    pub delivery_vtime_ns: u64,
    pub size: u32,
}

// Minimal manual generation of FlatBuffer bindings for TraceEvent
#[allow(dead_code, non_snake_case)]
pub mod telemetry_fb {
    use flatbuffers::{FlatBufferBuilder, WIPOffset};

    #[derive(Copy, Clone, PartialEq, Debug)]
    #[repr(i8)]
    pub enum TraceEventType {
        CpuState = 0,
        Irq = 1,
        Peripheral = 2,
    }

    pub struct TraceEventArgs<'a> {
        pub timestamp_ns: u64,
        pub type_: TraceEventType,
        pub id: u32,
        pub value: u32,
        pub device_name: Option<WIPOffset<&'a str>>,
    }

    pub fn create_trace_event<'a>(
        fbb: &mut FlatBufferBuilder<'a>,
        args: &TraceEventArgs<'a>,
    ) -> WIPOffset<flatbuffers::Table<'a>> {
        let start = fbb.start_table();
        fbb.push_slot(4, args.timestamp_ns, 0);
        fbb.push_slot(8, args.id, 0);
        fbb.push_slot(10, args.value, 0);
        if let Some(x) = args.device_name {
            fbb.push_slot_always(12, x);
        }
        fbb.push_slot(6, args.type_ as i8, 0);
        let end = fbb.end_table(start);
        WIPOffset::new(end.value())
    }
}

pub struct TraceEvent {
    pub timestamp_ns: u64,
    pub event_type: i8,
    pub id: u32,
    pub value: u32,
    pub device_name: Option<String>,
}
