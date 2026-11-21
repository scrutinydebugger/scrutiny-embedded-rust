#!/bin/bash
# Clones scrutiny-testapps and builds rust-testapp against this checkout of the library, all features enabled.
set -euo pipefail

APP_ROOT="$( cd "$( dirname "${BASH_SOURCE[0]}" )"/.. >/dev/null 2>&1 && pwd )"

TESTAPPS_REPO=${TESTAPPS_REPO:-https://github.com/scrutinydebugger/scrutiny-testapps}
TESTAPPS_BRANCH=${TESTAPPS_BRANCH:-rust-testapp}

LIB_GIT_URL="https://github.com/scrutinydebugger/scrutiny-embedded-rust"
LIB_NAME="scrutiny-embedded-rust"
TESTAPP_DIR=$(mktemp -d)
trap 'rm -rf "$TESTAPP_DIR"' EXIT

set -x
git clone --depth 1 --branch "$TESTAPPS_BRANCH" "$TESTAPPS_REPO" "$TESTAPP_DIR"
cd "$TESTAPP_DIR/rust-testapp"

# The testapp depends on the library through git. Redirect it to this checkout.
PATCH_ARG="patch.\"$LIB_GIT_URL\".$LIB_NAME.path=\"$APP_ROOT\""
FEATURES="$LIB_NAME/datalogging,$LIB_NAME/support_64bits,$LIB_NAME/protected_regions"

cargo build --config "$PATCH_ARG" --features "$FEATURES"

# A [patch] that does not match is only a warning, the build would silently use the GitHub version.
if ! cargo tree --config "$PATCH_ARG" -p "$LIB_NAME" --depth 0 | grep -qF "($APP_ROOT)"; then
    echo "rust-testapp was not built against the local library at $APP_ROOT"
    exit 1
fi
