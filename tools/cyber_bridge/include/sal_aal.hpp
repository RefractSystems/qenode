#pragma once

#include <cstdint>
#include <string>

namespace virtmcu {

// Base interface for Actuator Abstraction Layer
class Actuator {
public:
    virtual ~Actuator() = default;
    virtual std::string get_name() const = 0;
    virtual void apply_command(uint64_t vtime_ns, double command) = 0;
};

// Base interface for Sensor Abstraction Layer
class Sensor {
public:
    virtual ~Sensor() = default;
    virtual std::string get_name() const = 0;
    virtual double get_reading(uint64_t vtime_ns) = 0;
};

} // namespace virtmcu
