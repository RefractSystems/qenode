use byteorder::{LittleEndian, ReadBytesExt};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum ResdSampleType {
    Temperature = 0x0001,
    Acceleration = 0x0002,
    AngularRate = 0x0003,
    Voltage = 0x0004,
    Ecg = 0x0005,
    Humidity = 0x0006,
    Pressure = 0x0007,
    MagneticFluxDensity = 0x0008,
    BinaryData = 0x0009,
}

impl ResdSampleType {
    fn from_u16(val: u16) -> Option<Self> {
        match val {
            0x0001 => Some(Self::Temperature),
            0x0002 => Some(Self::Acceleration),
            0x0003 => Some(Self::AngularRate),
            0x0004 => Some(Self::Voltage),
            0x0005 => Some(Self::Ecg),
            0x0006 => Some(Self::Humidity),
            0x0007 => Some(Self::Pressure),
            0x0008 => Some(Self::MagneticFluxDensity),
            0x0009 => Some(Self::BinaryData),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResdSample {
    pub timestamp_ns: u64,
    pub data: Vec<i32>,
}

#[derive(Debug, Clone)]
pub struct ResdSensor {
    pub name: String,
    pub type_: ResdSampleType,
    pub samples: Vec<ResdSample>,
}

impl ResdSensor {
    pub fn new(name: String, type_: ResdSampleType) -> Self {
        Self {
            name,
            type_,
            samples: Vec::new(),
        }
    }

    pub fn last_timestamp(&self) -> u64 {
        self.samples.last().map(|s| s.timestamp_ns).unwrap_or(0)
    }

    pub fn get_reading(&self, vtime_ns: u64) -> Vec<f64> {
        if self.samples.is_empty() {
            return vec![0.0];
        }

        let mut idx = 0;
        while idx + 1 < self.samples.len() && self.samples[idx + 1].timestamp_ns <= vtime_ns {
            idx += 1;
        }

        if idx + 1 >= self.samples.len() || vtime_ns < self.samples[idx].timestamp_ns {
            // Zero-order hold
            return self.samples[idx].data.iter().map(|&x| x as f64).collect();
        }

        // Linear interpolation
        let s0 = &self.samples[idx];
        let s1 = &self.samples[idx + 1];

        let t0 = s0.timestamp_ns as f64;
        let t1 = s1.timestamp_ns as f64;
        let t = vtime_ns as f64;
        let factor = (t - t0) / (t1 - t0);

        s0.data
            .iter()
            .zip(s1.data.iter())
            .map(|(&v0, &v1)| v0 as f64 + factor * (v1 as f64 - v0 as f64))
            .collect()
    }
}

pub struct ResdParser {
    pub filename: String,
    pub sensors: HashMap<(ResdSampleType, u16), ResdSensor>,
}

impl ResdParser {
    pub fn new<P: AsRef<Path>>(filename: P) -> Self {
        Self {
            filename: filename.as_ref().to_string_lossy().into_owned(),
            sensors: HashMap::new(),
        }
    }

    pub fn init(&mut self) -> bool {
        self.parse().is_ok()
    }

    pub fn get_last_timestamp(&self) -> u64 {
        self.sensors
            .values()
            .map(|s| s.last_timestamp())
            .max()
            .unwrap_or(0)
    }

    fn parse(&mut self) -> std::io::Result<()> {
        let mut file = File::open(&self.filename)?;

        let mut magic = [0u8; 4];
        file.read_exact(&mut magic)?;
        if &magic != b"RESD" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid RESD magic",
            ));
        }

        let mut version = [0u8; 1];
        file.read_exact(&mut version)?;

        let mut padding = [0u8; 3];
        file.read_exact(&mut padding)?;

        loop {
            let mut block_type = [0u8; 1];
            if file.read_exact(&mut block_type).is_err() {
                break;
            }

            let sample_type_val = file.read_u16::<LittleEndian>()?;
            let channel_id = file.read_u16::<LittleEndian>()?;
            let data_size = file.read_u64::<LittleEndian>()?;

            let sample_type = match ResdSampleType::from_u16(sample_type_val) {
                Some(t) => t,
                None => {
                    file.seek(SeekFrom::Current(data_size as i64))?;
                    continue;
                }
            };

            let (start_time, period, subheader_size) = if block_type[0] == 0x01 {
                // ARBITRARY_TIMESTAMP
                (file.read_u64::<LittleEndian>()?, 0, 8)
            } else if block_type[0] == 0x02 {
                // CONSTANT_TIMESTAMP
                (
                    file.read_u64::<LittleEndian>()?,
                    file.read_u64::<LittleEndian>()?,
                    16,
                )
            } else {
                file.seek(SeekFrom::Current(data_size as i64))?;
                continue;
            };

            let metadata_size = file.read_u64::<LittleEndian>()?;
            file.seek(SeekFrom::Current(metadata_size as i64))?; // Skip metadata

            let samples_size = data_size - subheader_size - 8 - metadata_size;
            let mut bytes_read = 0;
            let mut current_time = start_time;

            let sensor = self
                .sensors
                .entry((sample_type, channel_id))
                .or_insert_with(|| {
                    ResdSensor::new(
                        format!("resd_{}_{}", sample_type as u16, channel_id),
                        sample_type,
                    )
                });

            while bytes_read < samples_size {
                let timestamp = if block_type[0] == 0x01 {
                    let ts = file.read_u64::<LittleEndian>()?;
                    bytes_read += 8;
                    ts
                } else {
                    current_time
                };

                let mut data = Vec::new();
                if sample_type == ResdSampleType::Temperature {
                    data.push(file.read_i32::<LittleEndian>()?);
                    bytes_read += 4;
                } else if sample_type == ResdSampleType::Acceleration
                    || sample_type == ResdSampleType::AngularRate
                {
                    data.push(file.read_i32::<LittleEndian>()?);
                    data.push(file.read_i32::<LittleEndian>()?);
                    data.push(file.read_i32::<LittleEndian>()?);
                    bytes_read += 12;
                } else {
                    file.seek(SeekFrom::Current((samples_size - bytes_read) as i64))?;
                    break;
                }

                sensor.samples.push(ResdSample {
                    timestamp_ns: timestamp,
                    data,
                });

                if block_type[0] == 0x02 {
                    current_time += period;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_resd_interpolation() {
        let mut s = ResdSensor::new("test".to_string(), ResdSampleType::Acceleration);
        s.samples.push(ResdSample {
            timestamp_ns: 1000,
            data: vec![100, 200, 300],
        });
        s.samples.push(ResdSample {
            timestamp_ns: 2000,
            data: vec![200, 400, 600],
        });

        let v1 = s.get_reading(500); // zero-order hold before
        assert_eq!(v1, vec![100.0, 200.0, 300.0]);

        let v2 = s.get_reading(1500); // 50%
        assert_eq!(v2, vec![150.0, 300.0, 450.0]);

        let v3 = s.get_reading(2000); // exact
        assert_eq!(v3, vec![200.0, 400.0, 600.0]);

        let v4 = s.get_reading(3000); // zero-order hold after
        assert_eq!(v4, vec![200.0, 400.0, 600.0]);
    }

    #[test]
    fn test_resd_malformed() {
        let mut p = ResdParser::new("nonexistent.resd");
        assert!(!p.init());

        // Create a truncated file
        let mut f = File::create("/tmp/trunc.resd").unwrap();
        f.write_all(b"RES").unwrap();
        drop(f);
        let mut p = ResdParser::new("/tmp/trunc.resd");
        assert!(!p.init());

        let mut f = File::create("/tmp/bad_magic.resd").unwrap();
        f.write_all(b"BADM\x01\x00\x00\x00").unwrap();
        drop(f);
        let mut p = ResdParser::new("/tmp/bad_magic.resd");
        assert!(!p.init());
    }
}
