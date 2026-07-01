#!/bin/sh
# Build and run the sqlly-datatable sample app.
#
# Usage:
#   ./run-sample.sh            # debug build, then run
#   ./run-sample.sh --release  # optimized build, then run
#   ./run-sample.sh --run-only # skip building, just run the existing binary
#
# Run from anywhere; the script always resolves paths relative to itself.

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$SCRIPT_DIR"

PROFILE=debug
CARGO_FLAGS=""
RUN_ONLY=0

for arg in "$@"; do
    case "$arg" in
        --release)
            PROFILE=release
            CARGO_FLAGS="--release"
            ;;
        --run-only)
            RUN_ONLY=1
            ;;
        *)
            echo "unknown option: $arg" >&2
            echo "usage: $0 [--release] [--run-only]" >&2
            exit 2
            ;;
    esac
done

BIN="target/$PROFILE/sqlly-datatable-sample"

if [ "$RUN_ONLY" -eq 0 ]; then
    echo "==> building ($PROFILE)..."
    cargo build -p sqlly-datatable-sample $CARGO_FLAGS
fi

if [ ! -x "$BIN" ]; then
    echo "error: binary not found at $BIN (build it first)" >&2
    exit 1
fi

echo "==> running $BIN"
exec "$BIN"
