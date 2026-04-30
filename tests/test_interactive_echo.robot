*** Settings ***
Documentation    Integration test for Phase 8 Interactive UART Echo Firmware.
Resource         ${CURDIR}/../tools/testing/qemu_keywords.robot
Test Setup       Launch And Connect
Test Teardown    Terminate Emulation

*** Variables ***
${DTB_PATH}      ${CURDIR}/../tests/fixtures/guest_apps/phase1/minimal.dtb
${FIRMWARE}      ${CURDIR}/../tests/fixtures/guest_apps/phase8/echo.elf

*** Keywords ***
Launch And Connect
    ${qmp}    ${uart}=    Launch Qemu    ${DTB_PATH}    ${FIRMWARE}    extra_args=-S
    Connect To Emulation    ${qmp}    ${uart}
    Start Emulation

*** Test Cases ***
Interactive Echo Should Work
    [Documentation]    Verify the firmware prints the welcome message and echoes input.
    
    # 1. Wait for welcome message
    Wait For Line On UART    Interactive UART Echo Ready.
    Wait For Line On UART    Type something:
    
    # 2. Type some characters
    Write To UART    Hello virtmcu\r
    
    # 3. Verify they are echoed back
    Wait For Line On UART    Hello virtmcu
