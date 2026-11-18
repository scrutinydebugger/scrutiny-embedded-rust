#!/bin/bash
# Sourced by build.sh and runtests.sh. Translates the C++-like SCRUTINY_* toggles into cargo arguments.

APP_ROOT="$( cd "$( dirname "${BASH_SOURCE[0]}" )"/.. >/dev/null 2>&1 && pwd )"

SCRUTINY_ENABLE_DATALOGGING=${SCRUTINY_ENABLE_DATALOGGING:-1}
SCRUTINY_SUPPORT_64BITS=${SCRUTINY_SUPPORT_64BITS:-1}
SCRUTINY_SUPPORT_PROTECTED_REGIONS=${SCRUTINY_SUPPORT_PROTECTED_REGIONS:-1}
SCRUTINY_TARGET=${SCRUTINY_TARGET:-}
BUILD_CONTEXT=${BUILD_CONTEXT:-}

export RUSTFLAGS="${RUSTFLAGS:--D warnings}"

# One target dir per context so parallel CI stages sharing a workspace do not collide.
if [ -n "$BUILD_CONTEXT" ]; then
    export CARGO_TARGET_DIR="$APP_ROOT/target/ci-$BUILD_CONTEXT"
fi

SCRUTINY_FEATURES=""
if [ "$SCRUTINY_ENABLE_DATALOGGING" = "1" ]; then
    SCRUTINY_FEATURES="$SCRUTINY_FEATURES,datalogging"
fi
if [ "$SCRUTINY_SUPPORT_64BITS" = "1" ]; then
    SCRUTINY_FEATURES="$SCRUTINY_FEATURES,support_64bits"
fi
if [ "$SCRUTINY_SUPPORT_PROTECTED_REGIONS" = "1" ]; then
    SCRUTINY_FEATURES="$SCRUTINY_FEATURES,protected_regions"
fi
SCRUTINY_FEATURES="${SCRUTINY_FEATURES#,}"

LIB_CARGO_ARGS=(--no-default-features)
if [ -n "$SCRUTINY_FEATURES" ]; then
    LIB_CARGO_ARGS+=(--features "$SCRUTINY_FEATURES")
fi

# The test harness requires std.
TEST_CARGO_ARGS=(--no-default-features)
if [ -n "$SCRUTINY_FEATURES" ]; then
    TEST_CARGO_ARGS+=(--features "std,$SCRUTINY_FEATURES")
else
    TEST_CARGO_ARGS+=(--features "std")
fi

if [ -n "$SCRUTINY_TARGET" ]; then
    LIB_CARGO_ARGS+=(--target "$SCRUTINY_TARGET")
    TEST_CARGO_ARGS+=(--target "$SCRUTINY_TARGET")
fi
