*** Settings ***
Documentation
...    Binary Fidelity Suite — verifies that unmodified firmware ELFs produce the same
...    observable behavior in VirtMCU as on real silicon.
...
...    Ground rules (from ADR-006):
...    - Firmware binaries live in tests/firmware/<target>/ and are built with a stock
...      ARM cross-compiler; no VirtMCU-specific flags, linker sections, or APIs.
...    - Each binary has a SHA256 entry in tests/firmware/SHA256SUMS so CI detects
...      accidental binary substitution.
...    - Expected UART output is in tests/firmware/<target>/golden_uart.txt.
...      Provenance (virtmcu-baseline vs silicon-validated) is in PROVENANCE.md.
...    - Skip lines starting with '#' in golden files (provenance comments).
...
...    Adding a new target:
...    1. Build firmware with standard cross-compiler, validate on real silicon, capture
...       UART output. Use tests/firmware/capture_golden.sh for VirtMCU-baseline capture.
...    2. Drop the ELF into tests/firmware/<target>/, add SHA256 to SHA256SUMS.
...    3. Write tests/firmware/<target>/PROVENANCE.md and golden_uart.txt.
...    4. Add a test case below following the pattern of the existing ones.

Library          Process
Library          OperatingSystem
Library          String
Resource         ${CURDIR}/../../../tools/testing/qemu_keywords.robot
Test Teardown    Terminate Emulation

*** Variables ***
${FIRMWARE_DIR}     ${CURDIR}/../../firmware

*** Keywords ***
Verify Binary Fidelity
    [Documentation]
    ...    Boots ${elf} on the VirtMCU described by ${dtb} and asserts that every
    ...    non-comment line in ${golden} appears on UART in order.
    ...    Fails if the SHA256 of any binary in FIRMWARE_DIR/SHA256SUMS does not match.
    [Arguments]    ${dtb}    ${elf}    ${golden}
    # Integrity check: every ELF listed in SHA256SUMS must match.
    # --ignore-missing allows files not yet added to SHA256SUMS (new targets in-flight),
    # but any file that IS listed must match exactly.
    ${result}=    Run Process    sha256sum    --check    --ignore-missing
    ...    ${FIRMWARE_DIR}/SHA256SUMS
    ...    cwd=${FIRMWARE_DIR}
    Should Be Equal As Integers    ${result.rc}    0
    ...    msg=SHA256 mismatch — ELF was replaced without re-validation. See ADR-006.
    # Boot the firmware
    ${qmp}    ${uart}=    Launch Qemu    ${dtb}    ${elf}    extra_args=-S
    Connect To Emulation    ${qmp}    ${uart}
    Start Emulation
    # Assert each non-comment line of the golden file appears on UART in order
    ${content}=    Get File    ${golden}
    FOR    ${line}    IN    @{content.splitlines()}
        ${stripped}=    Strip String    ${line}
        Continue For Loop If    not $stripped or $stripped.startswith('#')
        Wait For Line On UART    ${stripped}    timeout=10.0
    END

*** Test Cases ***
# ---------------------------------------------------------------------------
# Cortex-A15 / arm-generic-fdt — echo firmware
# Provenance: virtmcu-baseline (silicon validation pending — see PROVENANCE.md)
# Build:      arm-none-eabi-gcc -mcpu=cortex-a15 -nostdlib -T echo.ld echo.S -o echo.elf
# ---------------------------------------------------------------------------
Cortex-A15 Echo Firmware Runs Unmodified
    [Documentation]
    ...    Boots the echo firmware (standard arm-none-eabi build, no VirtMCU flags)
    ...    and asserts it produces the expected UART banner. The same ELF must also boot
    ...    on real Cortex-A15 hardware — see tests/firmware/cortex-a15-virt/PROVENANCE.md
    ...    for silicon validation status and instructions.
    [Tags]    binary-fidelity    cortex-a15    virtmcu-baseline
    Verify Binary Fidelity
    ...    dtb=${CURDIR}/../../fixtures/guest_apps/boot_arm/minimal.dtb
    ...    elf=${FIRMWARE_DIR}/cortex-a15-virt/echo.elf
    ...    golden=${FIRMWARE_DIR}/cortex-a15-virt/golden_uart.txt
