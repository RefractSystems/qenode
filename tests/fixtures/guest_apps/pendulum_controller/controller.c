#include <stdint.h>
#include "actuator.h"
#include "sensor.h"

#define UART0_BASE 0x09000000

void uart_putc(char c) { *(volatile uint32_t *)UART0_BASE = c; }
void uart_puts(const char *s) { while (*s) uart_putc(*s++); }

// Read sensor 0 (blocking until ready)
double read_sensor() {
    uart_puts("Reading sensor...\n");
    *(volatile uint32_t *)REG_SENSOR_ID = 0;
    *(volatile uint32_t *)REG_SENSOR_DATA_SIZE = 1;
    
    // Wait for new data to arrive in the shared buffer
    while (*(volatile uint32_t *)REG_SENSOR_READY == 0) {
        // asm volatile("wfi");
    }
    
    // Latch the data into peripheral registers
    *(volatile uint32_t *)REG_SENSOR_GO = 1;
    
    return *(volatile double *)REG_SENSOR_DATA;
}

// Write actuator 0
void write_actuator(double torque) {
    uart_puts("Torque: ");
    *(volatile uint32_t *)REG_ACTUATOR_ID = 0;
    *(volatile uint32_t *)REG_ACTUATOR_DATA_SIZE = 1;
    *(volatile double *)REG_ACTUATOR_DATA = torque;
    *(volatile uint32_t *)REG_ACTUATOR_GO = 1;
}

int main() {
    // Early test write to actuator
    *(volatile uint32_t *)REG_ACTUATOR_DATA_SIZE = 0xAA;

    uart_puts("Pendulum PID Controller Starting...\n");
    
    int prev_error = 0;
    int Kp = 50;
    int Kd = 10;
    
    uart_puts("Entering main loop...\n");
    while (1) {
        uart_puts("Calling read_sensor()...\n\n");
        double angle_rad = read_sensor();
        uart_puts("read_sensor() returned\n");
        
        // Angle in milli-radians (0.5 rad = 500 mrad)
        int angle = (int)(angle_rad * 1000.0);
        int error = 0 - angle;
        
        int derivative = error - prev_error;
        prev_error = error;
        
        int torque_milli = (Kp * error) + (Kd * derivative);
        
        uart_puts("Calling write_actuator()...\n");
        write_actuator((double)torque_milli / 1000.0);
        uart_puts("write_actuator() returned\n");
        
        uart_puts("Angle: ");
        if (angle < 0) { uart_putc('-'); angle = -angle; }
        char buf[16];
        int i = 0;
        if (angle == 0) buf[i++] = '0';
        while (angle > 0) {
            buf[i++] = '0' + (angle % 10);
            angle /= 10;
        }
        while (i > 0) {
            uart_putc(buf[--i]);
        }
        uart_puts("\n");
    }
    return 0;
}
