#pragma once

#include <cstdint>
#include <string>
#include <vector>

namespace virtmcu {

// Base interface for Actuator Abstraction Layer (AAL)
class Actuator {
public:
    virtual ~Actuator() = default;

    // Get the name of the actuator (e.g., "motor_fl")
    virtual std::string get_name() const = 0;

    // Apply a command received from firmware at a specific virtual time
    virtual void apply_command(uint64_t vtime_ns, const std::vector<double>& values) = 0;
};

// Base interface for Sensor Abstraction Layer (SAL)
class Sensor {
public:
    virtual ~Sensor() = default;

    // Get the name of the sensor (e.g., "imu0")
    virtual std::string get_name() const = 0;

    // Get the reading at a specific virtual time (can interpolate if needed)
    virtual std::vector<double> get_reading(uint64_t vtime_ns) = 0;
};

// Interface for a simulation backend (e.g., MuJoCo, RESD Replay)
class SimulationBackend {
public:
    virtual ~SimulationBackend() = default;

    // Initialize the backend
    virtual bool init() = 0;

    // Step the simulation to the target virtual time
    virtual void step_to(uint64_t vtime_ns) = 0;

    // Register a sensor with the backend
    virtual void register_sensor(Sensor* sensor) = 0;

    // Register an actuator with the backend
    virtual void register_actuator(Actuator* actuator) = 0;
};

} // namespace virtmcu
