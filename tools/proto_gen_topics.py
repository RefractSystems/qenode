#!/usr/bin/env python3
import re
import sys
import tomllib
from pathlib import Path
from typing import Any


def generate_python(config: dict[str, Any]) -> str:
    lines = [
        '"""',
        "AUTO-GENERATED from topics.toml. DO NOT EDIT MANUALLY.",
        '"""',
        "from __future__ import annotations",
        "from typing import Final",
        "",
        "",
        "class SimTopic:",
        '    """Enterprise topic registry for the VirtMCU simulation."""',
        "",
    ]

    lines.append("    # Singleton control-plane topics")
    for name, value in config["singleton"].items():
        lines.append(f'    {name}: Final[str] = "{value}"')
    lines.append("")

    lines.append("    # Wildcard subscriber patterns")
    for name, value in config["wildcard"].items():
        lines.append(f'    {name}: Final[str] = "{value}"')
    lines.append("")

    lines.append("    # Templates")
    for name, value in config["templates"].items():
        placeholders = re.findall(r"\{([a-z_]+)\}", value)
        args = ", ".join(f"{p}: int | str" for p in placeholders)
        lines.append("    @staticmethod")
        lines.append(f"    def {name}({args}) -> str:")
        lines.append(f'        return f"{value}"'.replace("{", "{").replace("}", "}"))
        lines.append("")

    return "\n".join(lines)


def generate_rust(config: dict[str, Any]) -> str:
    lines = [
        "// AUTO-GENERATED from topics.toml. DO NOT EDIT MANUALLY.",
        "#![allow(dead_code)]",
        "",
        "pub mod singleton {",
    ]
    for name, value in config["singleton"].items():
        lines.append(f'    pub const {name}: &str = "{value}";')
    lines.append("}")
    lines.append("")

    lines.append("pub mod wildcard {")
    for name, value in config["wildcard"].items():
        lines.append(f'    pub const {name}: &str = "{value}";')
    lines.append("}")
    lines.append("")

    lines.append("pub const ALL_LEGACY_TX_WILDCARDS: &[&str] = &[")
    for name in config["wildcard"]:
        if name.endswith("_TX_WILDCARD") and name != "COORD_TX_WILDCARD":
            lines.append(f"    wildcard::{name},")
    lines.append("];")
    lines.append("")

    lines.append("pub mod templates {")
    for name, value in config["templates"].items():
        val = (
            value.replace("{node_id}", "{}")
            .replace("{unique_id}", "{}")
            .replace("{plugin}", "{}")
            .replace("{suffix}", "{}")
            .replace("{bus}", "{}")
            .replace("{port_id}", "{}")
        )
        placeholders = re.findall(r"\{([a-z_]+)\}", value)
        args = ", ".join(f"{p}: &str" for p in placeholders)
        format_args = ", ".join(placeholders)
        lines.append(f"    pub fn {name}({args}) -> String {{")
        lines.append(f'        format!("{val}", {format_args})')
        lines.append("    }")
    lines.append("}")

    return "\n".join(lines)


def main() -> None:
    workspace = Path(__file__).parent.parent
    toml_path = workspace / "tools/deterministic_coordinator/protocol/topics.toml"

    with open(toml_path, "rb") as f:
        config = tomllib.load(f)

    py_content = generate_python(config)
    py_path = workspace / "tools/testing/virtmcu_test_suite/topics.py"
    with open(py_path, "w") as f:
        f.write(py_content)
    sys.stdout.write(f"Generated {py_path}\n")

    rs_content = generate_rust(config)
    rs_path = workspace / "tools/deterministic_coordinator/src/topics.rs"
    with open(rs_path, "w") as f:
        f.write(rs_content)
    sys.stdout.write(f"Generated {rs_path}\n")


if __name__ == "__main__":
    main()
