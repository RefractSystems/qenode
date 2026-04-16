#!/bin/bash
rustc --crate-type=staticlib --crate-name=rust_dummy --edition=2021 -C panic=abort -O hw/rust-dummy/src/lib.rs -o librust_dummy.a
