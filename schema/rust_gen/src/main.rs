use std::fs;
use typify::{TypeSpace, TypeSpaceSettings};

fn main() {
    let content = fs::read_to_string("../world_schema.json").unwrap();
    let schema = serde_json::from_str::<schemars::schema::RootSchema>(&content).unwrap();

    let mut type_space = TypeSpace::new(TypeSpaceSettings::default().with_struct_builder(true));
    type_space.add_root_schema(schema).unwrap();

    let code = format!(
        "// Generated file, do not edit\n#![allow(warnings)]\n#![allow(clippy::all)]\nuse serde::{{Deserialize, Serialize}};\n\n{}",
        prettyplease::unparse(&syn::parse2::<syn::File>(type_space.to_stream()).unwrap())
    );

    fs::write("../../tools/deterministic_coordinator/src/generated/topology.rs", code).unwrap();
}
