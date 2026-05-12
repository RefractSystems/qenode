#include <stdint.h>

#define UART0_BASE 0x09000000
#define ACTUATOR_BASE 0x50000000
#define SENSOR_BASE   0x51000000

#define REG_ACTUATOR_ID (ACTUATOR_BASE + 0x00)
#define REG_ACT_DATA_SIZE (ACTUATOR_BASE + 0x04)
#define REG_ACT_GO      (ACTUATOR_BASE + 0x08)
#define REG_ACT_DATA    (ACTUATOR_BASE + 0x10)

#define REG_SENSOR_ID   (SENSOR_BASE + 0x00)
#define REG_SENS_DATA_SIZE (SENSOR_BASE + 0x04)
#define REG_SENS_GO     (SENSOR_BASE + 0x08)
#define REG_SENS_READY  (SENSOR_BASE + 0x0C)
#define REG_SENS_DATA   (SENSOR_BASE + 0x10)

void uart_putc(char c) { *(volatile uint32_t *)UART0_BASE = c; }
void uart_puts(const char *s) { while (*s) uart_putc(*s++); }

// Read sensor 0 (blocking until ready)
double read_sensor() {
    *(volatile uint32_t *)REG_SENSOR_ID = 0;
    *(volatile uint32_t *)REG_SENS_DATA_SIZE = 1;
    *(volatile uint32_t *)REG_SENS_GO = 1;
    
    // In virtmcu, sensor data arrives asynchronously via zenoh.
    // The device will assert READY when it has new data.
    while (*(volatile uint32_t *)REG_SENS_READY == 0) {
        asm volatile("wfi"); // Wait for interrupt / yield
    }
    
    return *(volatile double *)REG_SENS_DATA;
}

// Write actuator 0
void write_actuator(double torque) {
    *(volatile uint32_t *)REG_ACTUATOR_ID = 0;
    *(volatile uint32_t *)REG_ACT_DATA_SIZE = 1;
    *(volatile double *)REG_ACT_DATA = torque;
    *(volatile uint32_t *)REG_ACT_GO = 1;
}

int main() {
    uart_puts("Pendulum PID Controller Starting...\n");
    
    double Kp = 50.0;
    double Kd = 10.0;
    double prev_angle = 0.0;
    
    while (1) {
        double angle = read_sensor();
        double error = 0.0 - angle;
        
        double derivative = error - prev_angle;
        prev_angle = error;
        
        double torque = (Kp * error) + (Kd * derivative);
        
        write_actuator(torque);
        
        // Print roughly the angle (multiply by 1000 and cast to int to avoid printf float issues)
        int angle_int = (int)(angle * 1000.0);
        uart_puts("Angle: ");
        // basic itoa
        if (angle_int < 0) { uart_putc('-'); angle_int = -angle_int; }
        char buf[16];
        int i = 0;
        if (angle_int == 0) buf[i++] = '0';
        while (angle_int > 0) {
            buf[i++] = '0' + (angle_int % 10);
            angle_int /= 10;
        }
        while (i > 0) {
            uart_putc(buf[--i]);
        }
        uart_puts("\n");
    }
    return 0;
}
