#!/bin/sh
# Test the bounded proof without building/importing a whole router or OSM graph.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
SOURCE=${MOTIS_SOURCE:-"$ROOT/.transit/motis-strict-source"}
TEST_DIR=$(mktemp -d)
trap 'rm -rf "$TEST_DIR"' EXIT HUP INT TERM
"${CXX:-c++}" -std=c++20 -O2 -Wall -Wextra -Werror \
    -I "$SOURCE/deps/osr/include" "$ROOT/scripts/test-motis-component-probe.cc" \
    -o "$TEST_DIR/probe-test"
"$TEST_DIR/probe-test"
