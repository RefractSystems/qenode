#pragma once

#include "sal_aal.hpp"
#include <string>
#include <vector>
#include <map>
#include <fstream>
#include <memory>

namespace virtmcu {

enum class ResdSampleType : uint16_t {
    TEMPERATURE = 0x0001,
    ACCELERATION = 0x0002,
    ANGULAR_RATE = 0x0003,
    VOLTAGE = 0x0004,
    ECG = 0x0005,
    HUMIDITY = 0x0006,
    PRESSURE = 0x0007,
    MAGNETIC_FLUX_DENSITY = 0x0008,
    BINARY_DATA = 0x0009
};

struct ResdSample {
    uint64_t timestamp_ns;
    std::vector<int32_t> data;
};

class ResdSensor : public Sensor {
public:
    ResdSensor(std::string name, ResdSampleType type);
    ~ResdSensor() override = default;

    std::string get_name() const override { return name_; }
    std::vector<double> get_reading(uint64_t vtime_ns) override;

    void add_sample(uint64_t timestamp, const std::vector<int32_t>& data);

private:
    std::string name_;
    ResdSampleType type_;
    std::vector<ResdSample> samples_;
    size_t current_idx_;
};

class ResdParser : public SimulationBackend {
public:
    ResdParser(const std::string& filename);
    ~ResdParser() override = default;

    bool init() override;
    void step_to(uint64_t vtime_ns) override;

    void register_sensor(Sensor* sensor) override {}
    void register_actuator(Actuator* actuator) override {}

    std::shared_ptr<ResdSensor> get_sensor(ResdSampleType type, uint16_t channel);

private:
    bool parse();

    std::string filename_;
    std::map<std::pair<ResdSampleType, uint16_t>, std::shared_ptr<ResdSensor>> sensors_;
};

} // namespace virtmcu
