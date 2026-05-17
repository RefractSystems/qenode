#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::vec::Vec;
    use virtmcu_wire::*;

    struct MockTransport {
        arena: Mutex<[u8; 1024]>,
        published: Mutex<Vec<Vec<u8>>>,
    }

    impl MockTransport {
        fn new() -> Self {
            Self { arena: Mutex::new([0; 1024]), published: Mutex::new(Vec::new()) }
        }
    }

    impl DataTransport for MockTransport {
        fn publish(&self, _topic: &str, payload: &[u8]) -> Result<(), String> {
            self.published.lock().unwrap().push(payload.to_vec());
            Ok(())
        }

        fn reserve<'a>(
            &'a self,
            topic: &'a str,
            size: usize,
        ) -> Result<TransportReservation<'a>, TransportError> {
            // Unsafe required because MutexGuard drops at the end of scope,
            // but we want to simulate an Arena for the lifetime test.
            // DO NOT DO THIS IN PROD! This is just to test TransportReservation lifetimes.
            let ptr = {
                let mut guard = self.arena.lock().unwrap();
                guard.as_mut_ptr()
            };
            let buffer = unsafe { core::slice::from_raw_parts_mut(ptr, size) };

            Ok(TransportReservation::new(topic, buffer, move |vtime, seq| {
                // Mock commit just copies what was written to a local vec and publishes it
                let ptr = self.arena.lock().unwrap().as_ptr();
                let mock_buf = unsafe { core::slice::from_raw_parts(ptr, size) };

                let mut final_buf = vec![0u8; 16 + size];
                final_buf[0..8].copy_from_slice(&vtime.to_le_bytes());
                final_buf[8..16].copy_from_slice(&seq.to_le_bytes());
                final_buf[16..].copy_from_slice(mock_buf);
                self.publish(topic, &final_buf).map_err(TransportError::Other)
            }))
        }

        fn register_link(
            &self,
            _node_id: u32,
            _link_name: &str,
            _protocol: virtmcu_wire::Protocol,
            _role: virtmcu_wire::LinkRole,
        ) -> Result<u32, TransportError> {
            Ok(0)
        }

        fn reserve_link<'a>(
            &'a self,
            _link_id: u32,
            size: usize,
        ) -> Result<TransportReservation<'a>, TransportError> {
            #[allow(deprecated)] // virtmcu-allow: allow reasoning="Stage 1 stub"
            self.reserve("sim/ch/0/tx", size)
        }

        fn subscribe(&self, _topic: &str, _callback: DataCallback) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn test_transport_reservation_lifecycle() {
        let transport = MockTransport::new();

        let mut reservation = transport.reserve("sim/dummy/tx", 4).unwrap();

        // Write some payload data
        reservation.buffer_mut().copy_from_slice(&[0x11, 0x22, 0x33, 0x44]);

        // Commit the reservation
        let res = reservation.commit(1000, 42);
        assert!(res.is_ok());

        // Verify publish was called correctly
        let published = transport.published.lock().unwrap();
        assert_eq!(published.len(), 1);
        let frame = &published[0];

        // VTIME is 8 bytes, SEQ is 8 bytes, Payload is 4 bytes
        assert_eq!(frame[0..8], 1000u64.to_le_bytes());
        assert_eq!(frame[8..16], 42u64.to_le_bytes());
        assert_eq!(&frame[16..], &[0x11, 0x22, 0x33, 0x44]);

        // Note: The compiler statically verifies that we cannot use `reservation.buffer_mut()`
        // here because `reservation` was moved by `commit(self)`.
    }
}
