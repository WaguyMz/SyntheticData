#!/bin/bash
#
# Run datasynth generation with config and output directory.
#
# Usage:
#   ./scripts/run_generations.sh                         # config.yaml -> ./output
#   ./scripts/run_generations.sh config_manu.yaml        # config_manu.yaml -> ./output
#   ./scripts/run_generations.sh --demo                 # Demo preset -> ./output
#   ./scripts/run_generations.sh -c my.yaml -o out       # Custom config and output
#   ./scripts/run_generations.sh --validate              # Validate config only
#
# The binary is target/release/datasynth-data. To run as 'dsd' from repo root:
#   alias dsd='./target/release/datasynth-data'
#   dsd generate --config config_manu.yaml --output ./output
#

set -eo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

BIN="${REPO_ROOT}/target/release/datasynth-data"
CONFIG="${REPO_ROOT}/config.yaml"
OUTPUT="${REPO_ROOT}/output"
DEMO=false
VALIDATE=true

# Optional first arg: config file name (e.g. config_manu.yaml)
if [[ -n "${1:-}" && "$1" != --* && "$1" != -c ]]; then
    CONFIG="${REPO_ROOT}/$1"
    shift
fi

# Parse optional -c / -o / --demo / --validate
while [[ $# -gt 0 ]]; do
    case "$1" in
        -c) CONFIG="${REPO_ROOT}/$2"; shift 2 ;;
        -o) OUTPUT="${REPO_ROOT}/$2"; shift 2 ;;
        --demo) DEMO=true; shift ;;
        --validate) VALIDATE=true; shift ;;
        *) shift ;;
    esac
done

if [[ ! -x "$BIN" ]]; then
    echo "Binary not found. Build with: cargo build --release -p datasynth-cli"
    echo "  Then run: $BIN generate --config $CONFIG --output $OUTPUT"
    exit 1
fi

if [[ "$VALIDATE" == true && "$DEMO" != true ]]; then
    "$BIN" validate --config "$CONFIG"
fi

if [[ "$DEMO" == true ]]; then
    "$BIN" generate --demo --output "$OUTPUT"
else
    echo "Running: $BIN generate --config $CONFIG --output $OUTPUT"
    "$BIN" generate --config "$CONFIG" --output "$OUTPUT"
fi

