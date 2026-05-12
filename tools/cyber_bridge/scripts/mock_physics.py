import mmap
import os
import struct
import time
import math

SHM_NAME = "/dev/shm/virtmcu_mujoco_0"

def main():
    print(f"Waiting for {SHM_NAME} to be created by time-authority...")
    while not os.path.exists(SHM_NAME):
        time.sleep(0.5)

    with open(SHM_NAME, "r+b") as f:
        mm = mmap.mmap(f.fileno(), 0)
        print("Connected to SHM.")

        # Header:
        # [0:4] n_sensors (u32)
        # [4:8] n_actuators (u32)
        # [8:16] bridge_seq (u64)
        # [16:24] mujoco_seq (u64)
        # [24:24 + 8*n_sensors] sensors (f64)
        # [24 + 8*n_sensors : ...] actuators (f64)

        n_sensors, n_actuators = struct.unpack_from("<II", mm, 0)
        print(f"Sensors: {n_sensors}, Actuators: {n_actuators}")
        
        if n_sensors < 1 or n_actuators < 1:
            print("Need at least 1 sensor and 1 actuator!")
            return

        sensors_offset = 24
        actuators_offset = 24 + n_sensors * 8

        # Simple pendulum physics state
        angle = 0.5  # Start slightly off-center
        velocity = 0.0
        dt = 0.001   # 1ms quantum

        mujoco_seq = 0

        while True:
            bridge_seq = struct.unpack_from("<Q", mm, 8)[0]
            if bridge_seq > mujoco_seq:
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
                mujoco_seq = bridge_seq
                struct.pack_into("<Q", mm, 16, mujoco_seq)
            else:
                time.sleep(0.0001)

if __name__ == "__main__":
    main()
