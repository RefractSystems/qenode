import argparse
import os

from cmsis_svd.parser import SVDParser
from jinja2 import Environment, FileSystemLoader, select_autoescape


def get_base_type(description: str | None) -> str:
    """Fallback logic if standard tags aren't used, but we try to be robust."""
    desc = description.lower() if description else ""
    if "float" in desc or "rad" in desc or "nm" in desc:
        return "float"
    return "uint32_t"


def generate_header(svd_path: str, template_path: str, output_path: str) -> None:
    parser = SVDParser.for_xml_file(svd_path)
    device = parser.get_device()

    # Process data for template
    peripherals = []
    for periph in device.peripherals:
        registers = []
        for reg in periph.registers:
            # We enforce 32-bit alignment in the template/assertions
            # Determine C type based on description or standard tags if they existed
            c_type = get_base_type(reg.description)

            registers.append(
                {
                    "name": reg.name,
                    "description": reg.description,
                    "offset": reg.address_offset,
                    "c_type": c_type,
                }
            )

        peripherals.append(
            {
                "name": periph.name,
                "description": periph.description,
                "base_address": periph.base_address,
                "size": periph.address_blocks[0].size if periph.address_blocks else 0x1000,
                "registers": registers,
            }
        )

    env = Environment(
        loader=FileSystemLoader(os.path.dirname(template_path)),
        autoescape=select_autoescape(),
    )
    template = env.get_template(os.path.basename(template_path))

    rendered = template.render(device_name=device.name, peripherals=peripherals)

    with open(output_path, "w") as f:
        f.write(rendered)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Generate C header from SVD.")
    parser.add_argument("svd_path", help="Path to the SVD XML file.")
    parser.add_argument("template_path", help="Path to the Jinja2 template.")
    parser.add_argument("output_path", help="Path to output the C header.")
    args = parser.parse_args()

    generate_header(args.svd_path, args.template_path, args.output_path)
