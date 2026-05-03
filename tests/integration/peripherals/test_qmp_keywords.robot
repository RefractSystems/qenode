*** Settings ***
Documentation    Integration test for virtmcu QMP keywords.
Resource        ../../../tools/testing/qemu_keywords.robot
Test Teardown   Terminate Emulation

*** Variables ***
${DTB}          ${CURDIR}/../../fixtures/guest_apps/boot_arm/minimal.dtb
${KERNEL}       ${CURDIR}/../../fixtures/guest_apps/boot_arm/hello.elf

*** Test Cases ***
Should Boot And Print HI
    [Documentation]    Starts QEMU and waits for "HI" on UART.
    ${qmp}    ${uart}=    Launch Qemu    ${DTB}    ${KERNEL}    extra_args=-S
    Connect To Emulation    ${qmp}    ${uart}
    
    # Verify it's paused
    ${res}=    Execute Monitor Command    info status
    Should Contain    ${res}    paused
    
    Start Emulation
    
    # Verify it's running
    ${res}=    Execute Monitor Command    info status
    Should Contain    ${res}    running
    
    Wait For Line On UART    HI    timeout=5.0

Should Retrieve PC
    [Documentation]    Verifies that we can read the PC.
    ${qmp}    ${uart}=    Launch Qemu    ${DTB}    ${KERNEL}    extra_args=-S
    Connect To Emulation    ${qmp}    ${uart}
    
    PC Should Be Equal    0x40000000

Should Reset Emulation
    [Documentation]    Verifies that system reset works.
    ${qmp}    ${uart}=    Launch Qemu    ${DTB}    ${KERNEL}    extra_args=-S
    Connect To Emulation    ${qmp}    ${uart}
    
    Start Emulation
    Wait For Line On UART    HI    timeout=5.0
    
    Reset Emulation
    Start Emulation
    Wait For Line On UART    HI    timeout=5.0
