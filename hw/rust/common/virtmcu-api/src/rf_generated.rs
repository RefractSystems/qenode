// Manually generated FlatBuffers bindings for the virtmcu RF frame header.
//
// Schema (conceptual .fbs, not run through flatc):
//
//   namespace virtmcu.rf;
//   table RfHeader {
//     delivery_vtime_ns: uint64 = 0;
//     sequence_number:   uint64 = 0;
//     size:              uint32 = 0;
//     rssi:              int8   = 0;
//     lqi:               uint8  = 255;
//   }
//   root_type RfHeader;
//
// Using FlatBuffers instead of the legacy 14-byte packed C struct
// enables schema evolution: new optional fields can be added without breaking
// consumers that read only the fields they know about.
//
// Wire format: size-prefixed FlatBuffer (4-byte LE length prefix followed by
// the table bytes), so consumers can determine the header length from the wire.

#![allow(clippy::all)]
extern crate alloc;

use flatbuffers::FlatBufferBuilder;

pub mod rf_header {
    use super::FlatBufferBuilder;
    use alloc::vec::Vec;

    /// VTable slot offsets for RfHeader fields.
    pub const VT_DELIVERY_VTIME_NS: flatbuffers::VOffsetT = 4;
    pub const VT_SEQUENCE_NUMBER: flatbuffers::VOffsetT = 6;
    pub const VT_SIZE: flatbuffers::VOffsetT = 8;
    pub const VT_RSSI: flatbuffers::VOffsetT = 10;
    pub const VT_LQI: flatbuffers::VOffsetT = 12;

    /// Identifier used in size-prefixed buffers.
    const RF_HEADER_IDENTIFIER: Option<&'static str> = None;

    /// Read-accessor for a serialized `RfHeader` FlatBuffer table.
    pub struct RfHeader<'a> {
        pub _tab: flatbuffers::Table<'a>,
    }

    impl<'a> flatbuffers::Follow<'a> for RfHeader<'a> {
        type Inner = RfHeader<'a>;
        #[inline]
        unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {
            RfHeader { _tab: unsafe { flatbuffers::Table::new(buf, loc) } }
        }
    }

    impl<'a> RfHeader<'a> {
        #[inline]
        pub unsafe fn init_from_table(table: flatbuffers::Table<'a>) -> Self {
            RfHeader { _tab: table }
        }

        #[inline]
        pub fn delivery_vtime_ns(&self) -> u64 {
            unsafe { self._tab.get::<u64>(VT_DELIVERY_VTIME_NS, Some(0)) }.unwrap_or(0)
        }

        #[inline]
        pub fn sequence_number(&self) -> u64 {
            unsafe { self._tab.get::<u64>(VT_SEQUENCE_NUMBER, Some(0)) }.unwrap_or(0)
        }

        #[inline]
        pub fn size(&self) -> u32 {
            unsafe { self._tab.get::<u32>(VT_SIZE, Some(0)) }.unwrap_or(0)
        }

        #[inline]
        pub fn rssi(&self) -> i8 {
            unsafe { self._tab.get::<i8>(VT_RSSI, Some(0)) }.unwrap_or(0)
        }

        #[inline]
        pub fn lqi(&self) -> u8 {
            unsafe { self._tab.get::<u8>(VT_LQI, Some(255)) }.unwrap_or(255)
        }
    }

    /// Builder for constructing an `RfHeader` FlatBuffer table.
    pub struct RfHeaderBuilder<'a, 'b> {
        fbb_: &'b mut FlatBufferBuilder<'a>,
        start_: flatbuffers::WIPOffset<flatbuffers::TableUnfinishedWIPOffset>,
    }

    impl<'a: 'b, 'b> RfHeaderBuilder<'a, 'b> {
        #[inline]
        pub fn add_delivery_vtime_ns(&mut self, v: u64) {
            self.fbb_.push_slot::<u64>(VT_DELIVERY_VTIME_NS, v, 0);
        }

        #[inline]
        pub fn add_sequence_number(&mut self, v: u64) {
            self.fbb_.push_slot::<u64>(VT_SEQUENCE_NUMBER, v, 0);
        }

        #[inline]
        pub fn add_size(&mut self, v: u32) {
            self.fbb_.push_slot::<u32>(VT_SIZE, v, 0);
        }

        #[inline]
        pub fn add_rssi(&mut self, v: i8) {
            self.fbb_.push_slot::<i8>(VT_RSSI, v, 0);
        }

        #[inline]
        pub fn add_lqi(&mut self, v: u8) {
            self.fbb_.push_slot::<u8>(VT_LQI, v, 255);
        }

        #[inline]
        pub fn new(fbb: &'b mut FlatBufferBuilder<'a>) -> Self {
            let start = fbb.start_table();
            RfHeaderBuilder { fbb_: fbb, start_: start }
        }

        #[inline]
        pub fn finish(self) -> flatbuffers::WIPOffset<RfHeader<'a>> {
            let o = self.fbb_.end_table(self.start_);
            flatbuffers::WIPOffset::new(o.value())
        }
    }

    /// Serialize an `RfHeader` into a size-prefixed FlatBuffer.
    pub fn encode(
        delivery_vtime_ns: u64,
        sequence_number: u64,
        size: u32,
        rssi: i8,
        lqi: u8,
    ) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::with_capacity(64);
        let mut hdr = RfHeaderBuilder::new(&mut builder);
        hdr.add_delivery_vtime_ns(delivery_vtime_ns);
        hdr.add_sequence_number(sequence_number);
        hdr.add_size(size);
        hdr.add_rssi(rssi);
        hdr.add_lqi(lqi);
        let offset = hdr.finish();
        builder.finish_size_prefixed(offset, RF_HEADER_IDENTIFIER);
        builder.finished_data().to_vec()
    }

    /// Parse the first `RfHeader` from a size-prefixed FlatBuffer slice.
    /// Returns `None` if the buffer is too short or otherwise malformed.
    pub fn decode(buf: &[u8]) -> Option<(u64, u64, u32, i8, u8)> {
        match flatbuffers::size_prefixed_root::<RfHeader>(buf) {
            Ok(hdr) => Some((
                hdr.delivery_vtime_ns(),
                hdr.sequence_number(),
                hdr.size(),
                hdr.rssi(),
                hdr.lqi(),
            )),
            Err(_) => None,
        }
    }

    impl flatbuffers::Verifiable for RfHeader<'_> {
        #[inline]
        fn run_verifier(
            v: &mut flatbuffers::Verifier,
            pos: usize,
        ) -> Result<(), flatbuffers::InvalidFlatbuffer> {
            v.visit_table(pos)?
                .visit_field::<u64>("delivery_vtime_ns", VT_DELIVERY_VTIME_NS, false)?
                .visit_field::<u64>("sequence_number", VT_SEQUENCE_NUMBER, false)?
                .visit_field::<u32>("size", VT_SIZE, false)?
                .visit_field::<i8>("rssi", VT_RSSI, false)?
                .visit_field::<u8>("lqi", VT_LQI, false)?
                .finish();
            Ok(())
        }
    }

    /// Encoded size in bytes of a minimally filled RfHeader (no optional fields).
    /// Used by callers to check the payload prefix before attempting decode.
    ///
    /// The actual size may be larger when the builder includes alignment padding,
    /// but `size_prefixed_root` handles variable sizes correctly — this constant
    /// is the minimum valid byte count.
    pub const MIN_ENCODED_BYTES: usize = 4 /* size prefix */ + 4 /* root offset */;
}

#[cfg(test)]
mod tests {
    use super::rf_header;

    #[test]
    fn test_encode_decode_roundtrip() {
        let vtime: u64 = 123_456_789_000;
        let seq: u64 = 42;
        let size: u32 = 127;
        let rssi: i8 = -70;
        let lqi: u8 = 200;

        let buf = rf_header::encode(vtime, seq, size, rssi, lqi);
        let (v2, sq2, s2, r2, l2) =
            rf_header::decode(&buf).unwrap_or_else(|| std::process::abort()); // "decode failed");

        assert_eq!(v2, vtime);
        assert_eq!(sq2, seq);
        assert_eq!(s2, size);
        assert_eq!(r2, rssi);
        assert_eq!(l2, lqi);
    }

    #[test]
    fn test_default_field_values() {
        // lqi default is 255.  When the writer writes 255 the vtable entry is
        // elided (FlatBuffers stores defaults as absent).  The reader must
        // return 255 regardless.
        let buf = rf_header::encode(0, 0, 0, 0, 255);
        let (_, _, _, _, lqi) = rf_header::decode(&buf).unwrap_or_else(|| std::process::abort()); // "decode failed");
        assert_eq!(lqi, 255, "absent lqi field should return default 255");
    }

    #[test]
    fn test_schema_evolution_new_reader_old_writer() {
        // An old writer writes only delivery_vtime_ns and size (rssi=0, lqi=255
        // — both defaults, so elided).  A new reader that knows about rssi and
        // lqi must receive sensible defaults for those absent fields.
        let buf = rf_header::encode(999, 0, 42, 0, 255);
        let (vtime, seq, size, rssi, lqi) = rf_header::decode(&buf).unwrap();
        assert_eq!(vtime, 999);
        assert_eq!(seq, 0);
        assert_eq!(size, 42);
        assert_eq!(rssi, 0); // default
        assert_eq!(lqi, 255); // default
    }

    #[test]
    fn test_decode_rejects_empty_buffer() {
        assert!(rf_header::decode(&[]).is_none());
        assert!(rf_header::decode(&[0u8; 3]).is_none());
    }
}
