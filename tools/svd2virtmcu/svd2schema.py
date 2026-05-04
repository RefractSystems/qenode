import argparse
import json
import os

from cmsis_svd.model import SVDPeripheral, SVDRegister
from cmsis_svd.parser import SVDParser
from jinja2 import Environment, FileSystemLoader, select_autoescape


def generate_schema(svd_path: str, template_path: str, output_path: str, world_id: str = "robot_arm") -> None:
    parser = SVDParser.for_xml_file(svd_path)
    device = parser.get_device()

    controls = []
    telemetry = []

    for periph in device.peripherals:
        if not isinstance(periph, SVDPeripheral) or periph.registers is None:
            continue
        for reg in periph.registers:
            if not isinstance(reg, SVDRegister) or reg.name is None:
                continue
            # Better attribute matching using descriptions or names
            if "TARGET" in reg.name:
                idx = reg.name[-1]
                label_map = {"0": "Shoulder", "1": "Elbow", "2": "Wrist"}
                label = label_map.get(idx, reg.name)
                controls.append(
                    {
                        "id": f"{label.lower()}_target",
                        "label": f"{label} Target (rad)",
                        "offset": reg.address_offset,
                        "type": "register",
                        "min": -1.5,
                        "max": 1.5,
                        "step": 0.01,
                        "unit": "rad",
                        "default": 0.0,
                    }
                )
            elif "QPOS" in reg.name:
                idx = reg.name[-1]
                label_map = {"0": "Shoulder", "1": "Elbow", "2": "Wrist"}
                label = label_map.get(idx, reg.name)
                telemetry.append(
                    {
                        "id": f"{label.lower()}_angle",
                        "label": f"{label} Angle",
                        "topic": "sim/telemetry/physics",
                        "json_path": "value",
                        "type": "readout",
                        "unit": "rad",
                    }
                )

    env = Environment(
        loader=FileSystemLoader(os.path.dirname(template_path)),
        autoescape=select_autoescape(),
    )
    template = env.get_template(os.path.basename(template_path))

    rendered = template.render(
        world_id=world_id,
        node_id="cyber-arm-ctrl",
        controls=controls,
        telemetry=telemetry,
    )

    with open(output_path, "w") as f:
        # Re-parse and dump to ensure valid formatting
        data = json.loads(rendered)
        json.dump(data, f, indent=2)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Generate UI Schema from SVD.")
    parser.add_argument("svd_path", help="Path to the SVD XML file.")
    parser.add_argument("template_path", help="Path to the Jinja2 template.")
    parser.add_argument("output_path", help="Path to output the UI Schema JSON.")
    args = parser.parse_args()

    generate_schema(args.svd_path, args.template_path, args.output_path)
