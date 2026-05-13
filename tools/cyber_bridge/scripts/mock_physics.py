import mmap
import os
import struct
import time
import math

# SHM header layout (all u32, little-endian):
# [0:4]   n_sensors
# [4:8]   n_actuators
# [8:12]  bridge_seq   (gateway → physics)
# [12:16] physics_seq  (physics → gateway)
# [16:20] shutdown     (1 = exit)
# [20:24] reserved
# [24..]  sensor f64s, then actuator f64s
SHM_NAME        = "/dev/shm/virtmcu_physics_0"
OFF_N_SENSORS   = 0
OFF_N_ACTUATORS = 4
OFF_BRIDGE_SEQ  = 8
OFF_PHYSICS_SEQ = 12
OFF_SHUTDOWN    = 16
SHM_DATA_OFFSET = 24

def main():
    print(f"Waiting for {SHM_NAME} to be created by physical-node...")
    while not os.path.exists(SHM_NAME):
        time.sleep(0.5)

    with open(SHM_NAME, "r+b") as f:
        mm = mmap.mmap(f.fileno(), 0)
        print("Connected to SHM.")

        n_sensors, n_actuators = struct.unpack_from("<II", mm, 0)
        print(f"Sensors: {n_sensors}, Actuators: {n_actuators}")
        
        if n_sensors < 1 or n_actuators < 1:
            print("Need at least 1 sensor and 1 actuator!")
            return

        sensors_offset = SHM_DATA_OFFSET
        actuators_offset = SHM_DATA_OFFSET + n_sensors * 8

        # Simple pendulum physics state
        angle = 0.5  # Start slightly off-center
        velocity = 0.0
        dt = 0.001   # 1ms quantum

        physics_seq = 0

        while True:
            bridge_seq = struct.unpack_from("<I", mm, OFF_BRIDGE_SEQ)[0]
            if bridge_seq != physics_seq:
                # Check shutdown before doing any work
                shutdown = struct.unpack_from("<I", mm, OFF_SHUTDOWN)[0]
                if shutdown:
                    print("Shutdown requested. Exiting.")
                    break
                
                # Read actuator (torque)
                torque = struct.unpack_from("<d", mm, actuators_offset)[0]
                
                # Physics step
                gravity = 9.81
                length = 1.0
                damping = 0.1
                
                angular_accel = (torque - gravity / length * math.sin(angle) - damping * velocity)
                velocity += angular_accel * dt
                angle += velocity * dt
                
                # Write sensor (angle)
                struct.pack_into("<d", mm, sensors_offset, angle)
                
                # Acknowledge
                physics_seq = bridge_seq
                struct.pack_into("<I", mm, OFF_PHYSICS_SEQ, physics_seq)
            else:
                time.sleep(0.0001)

if __name__ == "__main__":
    main()
