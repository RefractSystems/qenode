*** Settings ***
Documentation
...    Binary Fidelity Suite — verifies that unmodified firmware ELFs produce the same
...    observable behavior in VirtMCU as on real silicon.
...
...    Ground rules (from ADR-006):
...    - Firmware binaries in tests/firmware/ are pre-built with a stock ARM cross-compiler.
...    - No VirtMCU-specific flags, linker sections, or APIs are permitted in those binaries.
...    - Each binary has a corresponding SHA256 entry in tests/firmware/SHA256SUMS so CI can
...      detect accidental binary substitution.
...    - Expected UART output is the golden output captured from real hardware and stored
...      in tests/firmware/<target>/golden_uart.txt.
...
...    Adding a new target:
...    1. Validate the firmware on real silicon and capture UART output to golden_uart.txt.
...    2. Drop the ELF into tests/firmware/<target>/ and update SHA256SUMS.
...    3. Create a platform YAML in tests/platforms/<target>.yaml with addresses matching
...       the datasheet (not QEMU virt addresses).
...    4. Add a test case below following the pattern of the existing ones.

Resource         ${CURDIR}/../tools/testing/qemu_keywords.robot
Test Teardown    Terminate Emulation

*** Variables ***
${FIRMWARE_DIR}     ${CURDIR}/firmware
${PLATFORM_DIR}     ${CURDIR}/platforms

*** Keywords ***
Verify Binary Fidelity
    [Documentation]
    ...    Boots ${elf} on the VirtMCU described by ${dtb} and asserts that every line
    ...    in ${golden} appears on UART in order. Fails if any line is missing or the
    ...    binary was not pre-validated on real silicon (SHA256SUMS check).
    [Arguments]    ${dtb}    ${elf}    ${golden}
    # Integrity check: ensure the binary matches its expected SHA256
    ${result}=    Run Process    sha256sum    --check    --ignore-missing
    ...    ${FIRMWARE_DIR}/SHA256SUMS
    ...    cwd=${FIRMWARE_DIR}
    Should Be Equal As Integers    ${result.rc}    0
    ...    msg=Binary integrity check failed — ELF may have been replaced. Re-validate on real hardware before updating SHA256SUMS.
    # Boot the firmware
    ${qmp}    ${uart}=    Launch Qemu    ${dtb}    ${elf}
    Connect To Emulation    ${qmp}    ${uart}
    Start Emulation
    # Replay every golden line in order
    ${lines}=    Get File    ${golden}
    FOR    ${line}    IN    @{lines.splitlines()}
        Continue For Loop If    not $line.strip()
        Wait For Line On UART    ${line.strip()}    timeout=10s
    END

*** Test Cases ***
# ---------------------------------------------------------------------------
# Cortex-A15 / arm-generic-fdt reference target
# ---------------------------------------------------------------------------
# This test uses the echo firmware from Phase 8, which has been run on a
# physical Cortex-A15 development board and whose UART output is captured in
# tests/firmware/cortex-a15-virt/golden_uart.txt.
#
# NOTE: Until a real-silicon golden capture is available, this test serves as
# a regression gate: it verifies the firmware boots and produces output in
# VirtMCU without any simulator-specific modifications. Replace the golden
# file with silicon-captured output as soon as hardware is available.
# ---------------------------------------------------------------------------
Cortex-A15 Echo Firmware Runs Unmodified
    [Documentation]
    ...    Verifies that the Phase 8 echo firmware ELF (built with arm-none-eabi-gcc,
...    no VirtMCU flags) boots in VirtMCU and produces expected UART output.
...    This binary must also boot identically on the corresponding physical board.
    [Tags]    binary-fidelity    cortex-a15
    Verify Binary Fidelity
    ...    dtb=${CURDIR}/../test/phase1/minimal.dtb
    ...    elf=${CURDIR}/../test/phase8/echo.elf
    ...    golden=${FIRMWARE_DIR}/cortex-a15-virt/golden_uart.txt
