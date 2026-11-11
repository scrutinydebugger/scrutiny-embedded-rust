#!/bin/bash
set -euo pipefail

cargo test
cargo clippy --all-features -- -D warnings