#!/bin/bash
set -euo pipefail

source "$( cd "$( dirname "${BASH_SOURCE[0]}" )" >/dev/null 2>&1 && pwd )/common.sh"

cd "$APP_ROOT"
set -x

cargo build --release "${LIB_CARGO_ARGS[@]}"
