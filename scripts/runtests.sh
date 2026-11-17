#!/bin/bash
set -euo pipefail

cargo test --features std
cargo clippy --all-features -- -D warnings