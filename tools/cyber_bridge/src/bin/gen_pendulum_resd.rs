use byteorder::{LittleEndian, WriteBytesExt};
use std::fs::File;
use std::io::Write;

fn main() {
    let path = "tests/fixtures/guest_apps/pendulum_controller/pendulum_angles.resd";
    let mut file = File::create(path).expect("Failed to create RESD file");

    // Header: "RESD", version 1, padding
    file.write_all(b"RESD").unwrap();
    file.write_u8(1).unwrap();
    file.write_all(&[0, 0, 0]).unwrap();

    // Block: CONSTANT_TIMESTAMP (0x02)
    file.write_u8(0x02).unwrap();
    // Sample Type: Double (0x000A)
    file.write_u16::<LittleEndian>(0x000A).unwrap();
    // Channel ID: 0
    file.write_u16::<LittleEndian>(0).unwrap();

    // Data Size calculation:
    // Subheader: Start Time (8) + Period (8) = 16
    // Metadata Size: 8
    // Metadata: 0
    // Samples: 21 samples * 8 bytes = 168
    // Total = 168 + 16 + 8 = 192
    let n_samples = 21;
    let data_size = 16 + 8 + (n_samples * 8);
    file.write_u64::<LittleEndian>(data_size as u64).unwrap();

    // Subheader: Start Time = 0, Period = 1ms (1,000,000 ns)
    file.write_u64::<LittleEndian>(0).unwrap();
    file.write_u64::<LittleEndian>(1_000_000).unwrap();

    // Metadata Size = 0
    file.write_u64::<LittleEndian>(0).unwrap();

    // Samples: angle_n = 0.5 * cos(n * 0.1), n = 0..20
    for n in 0..n_samples {
        let angle = 0.5 * (n as f64 * 0.1).cos();
        file.write_f64::<LittleEndian>(angle).unwrap();
    }

    println!("Successfully generated {}", path);
}
