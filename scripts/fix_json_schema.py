import json
from typing import Any

schema_path = "schema/world_schema.json"
with open(schema_path) as f:
    schema = json.load(f)

def fix_refs(obj: Any) -> None:  # noqa: ANN401
    if isinstance(obj, dict):
        if "$ref" in obj and not obj["$ref"].startswith("#"):
            # Machine.json -> #/$defs/Machine
            ref = obj["$ref"].replace(".yaml", "").replace(".json", "")
            obj["$ref"] = f"#/$defs/{ref}"
        for v in obj.values():
            fix_refs(v)
    elif isinstance(obj, list):
        for item in obj:
            fix_refs(item)

fix_refs(schema)

with open(schema_path, "w") as f:
    json.dump(schema, f, indent=2)
