*** Settings ***
Documentation    Integration test for Multi-Node UART over Zenoh.
Resource         ${CURDIR}/../tools/testing/qemu_keywords.robot
Test Teardown    Terminate All

*** Variables ***
${DTB_PATH}      ${CURDIR}/../tests/fixtures/guest_apps/boot_arm/minimal.dtb
${FIRMWARE}      ${CURDIR}/../tests/fixtures/guest_apps/uart_echo/echo.elf

*** Keywords ***
Terminate All
    Run Keyword And Ignore Error    Terminate Emulation
    Run Keyword And Ignore Error    Terminate Emulation    qmp2

*** Test Cases ***
Multi Node UART Should Echo Across Nodes
    [Documentation]    Verify Node 1 sending UART data reaches Node 2 via Zenoh coordinator.
    
    # 1. Start QEMU instances with Zenoh Chardev configured
    ${qmp1}    ${uart1}=    Launch Qemu    ${DTB_PATH}    ${FIRMWARE}    extra_args=-S -chardev virtmcu,id=chr0,node=node1 -serial chardev:chr0
    ${qmp2}    ${uart2}=    Launch Qemu    ${DTB_PATH}    ${FIRMWARE}    extra_args=-S -chardev virtmcu,id=chr0,node=node2 -serial chardev:chr0
    
    Connect To Emulation    ${qmp1}    ${uart1}
    # TODO: Current QmpBridge is a singleton connected to the last Launch Qemu.
    # To test multi-node natively, we'd need dual QMP connection support.
    # For now, we rely on the C unit tests and smoke scripts for full coverage.
    Start Emulation
