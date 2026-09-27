#!/bin/sh
# MOTIS 2.11.3 with OSR's synthetic sub-eight-metre WALK routes removed.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
SOURCE="$ROOT/.transit/motis-strict-source"
PATCH="$ROOT/scripts/motis-strict-streets.patch"

case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) PRESET=macos-arm64 ;;
    Darwin-x86_64) PRESET=macos-x86_64 ;;
    *) echo 'This local MOTIS build command requires macOS and CMake/Ninja/Clang.' >&2; exit 1 ;;
esac
if [ ! -d "$SOURCE" ]; then
    git clone --depth 1 --branch v2.11.3 https://github.com/motis-project/motis.git "$SOURCE"
fi
if [ "$(git -C "$SOURCE" rev-parse HEAD)" != b228a4519d196d9dd01b5ce80be46e642abc953e ]; then
    echo 'Unexpected MOTIS source revision; refusing to patch another version.' >&2
    exit 1
fi
cd "$SOURCE"
# Configure fetches the exact dependency revisions in MOTIS's .pkg.lock.
cmake --preset "$PRESET"
if [ "$(git -C deps/osr rev-parse HEAD)" != a7b2ec2728544304ef1d8397b3042abc8d10f7e7 ]; then
    echo 'Unexpected OSR source revision; refusing to patch another version.' >&2
    exit 1
fi
if git -C deps/osr apply --reverse --check "$PATCH" 2>/dev/null; then
    : # Already patched; an incremental build is safe.
else
    git -C deps/osr apply --check "$PATCH"
    git -C deps/osr apply "$PATCH"
fi
cmake --build "build/$PRESET-release" --target motis --parallel "${MOTIS_BUILD_JOBS:-2}"
# Keep the vendor distribution (including UI assets) and old binary untouched.
mkdir -p "$ROOT/.transit/motis-strict"
cp "build/$PRESET-release/motis" "$ROOT/.transit/motis-strict/motis.new"
mv "$ROOT/.transit/motis-strict/motis.new" "$ROOT/.transit/motis-strict/motis"
echo "Strict graph-routing MOTIS installed: $ROOT/.transit/motis-strict/motis"
