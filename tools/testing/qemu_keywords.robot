*** Settings ***
Documentation    Robot Framework resource file for QEMU emulation testing.
...              Provides Renode-compatible keywords backed by QEMU's QMP.
Library          ${CURDIR}/QemuLibrary.py

*** Keywords ***
Launch Qemu
    [Arguments]    ${dtb_path}    ${kernel_path}=${None}    ${extra_args}=${None}
    [Documentation]    Launches QEMU via run.sh and returns (qmp_sock, uart_sock) paths.
    ${qmp}    ${uart}=    QemuLibrary.Launch Qemu    ${dtb_path}    ${kernel_path}    ${extra_args}
    RETURN    ${qmp}    ${uart}

Connect To Emulation
    [Arguments]    ${qmp_sock}    ${uart_sock}=${None}
    QemuLibrary.Connect To Qemu    ${qmp_sock}    ${uart_sock}

Start Emulation
    QemuLibrary.Start Emulation

Pause Emulation
    QemuLibrary.Pause Emulation

Reset Emulation
    QemuLibrary.Reset Emulation

Wait For Line On UART
    [Arguments]    ${pattern}    ${timeout}=10.0
    QemuLibrary.Wait For Line On Uart    ${pattern}    ${timeout}

Write To UART
    [Arguments]    ${text}
    [Documentation]    Writes the given text string to the primary UART.
    QemuLibrary.Write To Uart    ${text}

PC Should Be Equal
    [Arguments]    ${expected_addr}
    QemuLibrary.Pc Should Be Equal    ${expected_addr}

Execute Monitor Command
    [Arguments]    ${cmd}
    ${output}=    QemuLibrary.Execute Monitor Command    ${cmd}
    RETURN    ${output}

Load ELF
    [Arguments]    ${path}
    Log    Load ELF is typically handled via QEMU CLI --kernel. QMP loading deferred.    WARN

Terminate Emulation
    QemuLibrary.Close All Connections
