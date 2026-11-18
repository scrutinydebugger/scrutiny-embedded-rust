#!/bin/bash
set -euo pipefail

source "$( cd "$( dirname "${BASH_SOURCE[0]}" )" >/dev/null 2>&1 && pwd )/common.sh"

cd "$APP_ROOT"
set -x

cargo test "${TEST_CARGO_ARGS[@]}"
cargo clippy "${LIB_CARGO_ARGS[@]}" -- -D warnings
