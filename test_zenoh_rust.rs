use zenoh::prelude::sync::*;

fn main() {
    let mut config = zenoh::Config::default();
    let session = zenoh::open(config).res_sync().unwrap();
}
