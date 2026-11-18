#!/bin/bash
set -euo pipefail

APP_ROOT="$( cd "$( dirname "${BASH_SOURCE[0]}" )"/.. >/dev/null 2>&1 && pwd )"
cd "$APP_ROOT"

if [ -n "${BUILD_CONTEXT:-}" ]; then
    export CARGO_TARGET_DIR="$APP_ROOT/target/ci-$BUILD_CONTEXT"
fi

set -x

cargo fmt --all -- --check
cargo clippy --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
