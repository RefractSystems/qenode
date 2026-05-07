import json
import sys
from pathlib import Path
from typing import Any

schema_path = Path(sys.argv[1] if len(sys.argv) > 1 else "schema/world_schema.json")

schema = json.loads(schema_path.read_text())


def fix_refs(obj: Any) -> None:
    if isinstance(obj, dict):
        if "$ref" in obj and isinstance(obj["$ref"], str) and not obj["$ref"].startswith("#"):
            # Machine.json -> #/$defs/Machine
            ref = obj["$ref"].replace(".yaml", "").replace(".json", "")
            obj["$ref"] = f"#/$defs/{ref}"
        for v in obj.values():
            fix_refs(v)
    elif isinstance(obj, list):
        for item in obj:
            fix_refs(item)


fix_refs(schema)

schema_path.write_text(json.dumps(schema, indent=2))
