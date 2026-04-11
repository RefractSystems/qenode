#include "virtmcu/resd_parser.hpp"
#include <iostream>
#include <fstream>
#include <cassert>

namespace virtmcu {

ResdSensor::ResdSensor(std::string name, ResdSampleType type)
    : name_(std::move(name)), type_(type), current_idx_(0) {}

std::vector<double> ResdSensor::get_reading(uint64_t vtime_ns) {
    if (samples_.empty()) {
        return {0.0};
    }

    // Fast-forward to the current sample
    while (current_idx_ + 1 < samples_.size() && samples_[current_idx_ + 1].timestamp_ns <= vtime_ns) {
        current_idx_++;
    }

    // Basic zero-order hold interpolation
    std::vector<double> ret;
    for (int32_t val : samples_[current_idx_].data) {
        ret.push_back(static_cast<double>(val));
    }
    return ret;
}

void ResdSensor::add_sample(uint64_t timestamp, const std::vector<int32_t>& data) {
    samples_.push_back({timestamp, data});
}

ResdParser::ResdParser(const std::string& filename) : filename_(filename) {}

bool ResdParser::init() {
    return parse();
}

void ResdParser::step_to(uint64_t vtime_ns) {
    // RESD parsing is typically offline and fully loaded in memory for standalone.
    // Time stepping is just querying the correct index.
}

std::shared_ptr<ResdSensor> ResdParser::get_sensor(ResdSampleType type, uint16_t channel) {
    auto it = sensors_.find({type, channel});
    if (it != sensors_.end()) return it->second;
    
    // Create lazily if not found during parsing
    auto sensor = std::make_shared<ResdSensor>("resd_" + std::to_string((int)type) + "_" + std::to_string(channel), type);
    sensors_[{type, channel}] = sensor;
    return sensor;
}

bool ResdParser::parse() {
    std::ifstream file(filename_, std::ios::binary);
    if (!file.is_open()) {
        std::cerr << "[RESD] Failed to open " << filename_ << std::endl;
        return false;
    }

    // Read header: 4 bytes "RESD", 1 byte version, 3 bytes padding
    char magic[4];
    file.read(magic, 4);
    if (std::string(magic, 4) != "RESD") {
        std::cerr << "[RESD] Invalid magic" << std::endl;
        return false;
    }

    uint8_t version;
    file.read(reinterpret_cast<char*>(&version), 1);
    file.seekg(3, std::ios::cur); // padding

    while (file.peek() != EOF) {
        uint8_t block_type;
        uint16_t sample_type;
        uint16_t channel_id;
        uint64_t data_size;

        if (!file.read(reinterpret_cast<char*>(&block_type), 1)) break;
        file.read(reinterpret_cast<char*>(&sample_type), 2);
        file.read(reinterpret_cast<char*>(&channel_id), 2);
        file.read(reinterpret_cast<char*>(&data_size), 8);

        auto sensor = get_sensor(static_cast<ResdSampleType>(sample_type), channel_id);

        uint64_t start_time = 0;
        uint64_t period = 0;
        uint64_t subheader_size = 0;

        if (block_type == 0x01) { // ARBITRARY_TIMESTAMP
            file.read(reinterpret_cast<char*>(&start_time), 8);
            subheader_size = 8;
        } else if (block_type == 0x02) { // CONSTANT_FREQUENCY
            file.read(reinterpret_cast<char*>(&start_time), 8);
            file.read(reinterpret_cast<char*>(&period), 8);
            subheader_size = 16;
        } else {
            // Unknown block type
            file.seekg(data_size, std::ios::cur);
            continue;
        }

        // Read metadata
        uint64_t metadata_size;
        file.read(reinterpret_cast<char*>(&metadata_size), 8);
        file.seekg(metadata_size, std::ios::cur); // Skip metadata for now

        uint64_t samples_size = data_size - subheader_size - 8 - metadata_size;
        uint64_t bytes_read = 0;

        uint64_t current_time = start_time;

        while (bytes_read < samples_size) {
            uint64_t timestamp = current_time;
            if (block_type == 0x01) {
                file.read(reinterpret_cast<char*>(&timestamp), 8);
                bytes_read += 8;
            }

            std::vector<int32_t> data;
            // Parse based on sample_type
            if (sample_type == 0x0001) { // TEMPERATURE
                int32_t temp;
                file.read(reinterpret_cast<char*>(&temp), 4);
                data.push_back(temp);
                bytes_read += 4;
            } else if (sample_type == 0x0002 || sample_type == 0x0003) { // ACCELERATION, ANGULAR_RATE
                int32_t x, y, z;
                file.read(reinterpret_cast<char*>(&x), 4);
                file.read(reinterpret_cast<char*>(&y), 4);
                file.read(reinterpret_cast<char*>(&z), 4);
                data.push_back(x);
                data.push_back(y);
                data.push_back(z);
                bytes_read += 12;
            } else {
                // Not supported, skip to end of block
                file.seekg(samples_size - bytes_read, std::ios::cur);
                break;
            }

            sensor->add_sample(timestamp, data);
            
            if (block_type == 0x02) {
                current_time += period;
            }
        }
    }

    return true;
}

} // namespace virtmcu
