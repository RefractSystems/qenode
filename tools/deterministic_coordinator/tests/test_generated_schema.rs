use std::fs;
use std::path::Path;

// This will verify that the generated typify bindings can successfully deserialize
// our test topologies exactly like the Pydantic tests do.

// Load the generated topology struct
use deterministic_coordinator::generated::topology::WorldSchema;

#[test]
fn test_parse_test_board_yaml() {
    let yaml_path = Path::new("../../tests/fixtures/guest_apps/yaml_boot/test_board.yaml");
    assert!(yaml_path.exists(), "Test board YAML not found");

    let content = fs::read_to_string(yaml_path).unwrap();
    let world: WorldSchema = serde_yaml::from_str(&content).unwrap();

    let machine = world.machine.clone().unwrap();
    assert_eq!(machine.name.clone().unwrap(), "test_board");
    assert_eq!(machine.type_.clone().unwrap(), "arm-generic-fdt");

    let cpus = machine.cpus.clone();
    assert_eq!(cpus.len(), 1);
    assert_eq!(cpus[0].name.clone(), "cpu");
    assert_eq!(cpus[0].type_.clone(), "cortex-a15");
    assert_eq!(cpus[0].memory.clone().unwrap(), "sysmem");

    let peripherals = world.peripherals.clone();
    assert_eq!(peripherals.len(), 3);

    let uart = peripherals
        .iter()
        .find(|p| p.name.to_string() == "uart0")
        .unwrap();
    assert_eq!(uart.renode_type.clone().unwrap(), "UART.PL011");

    // Check address and interrupts
    // Address is generated as an enum (Address::String or Address::Integer)
    match uart.address.clone().unwrap() {
        deterministic_coordinator::generated::topology::Address::String(s) => {
            assert_eq!(s.as_str(), "0x09000000");
        }
        _ => panic!("Expected Address::String for uart0"),
    }
}

#[test]
fn test_parse_lin_topology() {
    let yaml_path = Path::new("../../tests/fixtures/topologies/lin_2node.yml");
    assert!(yaml_path.exists(), "LIN topology YAML not found");

    let content = fs::read_to_string(yaml_path).unwrap();
    let world: WorldSchema = serde_yaml::from_str(&content).unwrap();

    let topology = world.topology.clone().unwrap();
    let nodes = topology.nodes.clone();
    assert_eq!(nodes.len(), 2);

    // NodeId is generated as an enum similar to Address
    assert_eq!(nodes[0].name.to_string(), "0");

    let links = topology.links.clone();
    assert_eq!(links.len(), 1);

    let link = &links[0];
    assert_eq!(link.type_.clone().as_str(), "lin");

    assert_eq!(link.nodes[0].to_string(), "0");
}
