#!/bin/sh
# Installed local Moscow deployment; keep large inputs and indexes in .transit/.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
MOTIS="$ROOT/.transit/motis-strict/motis"
DATA="$ROOT/.transit/moscow"

case "${1:-app}" in
    router|import)
        if [ ! -x "$MOTIS" ] || [ ! -f "$DATA/config.yml" ]; then
            echo 'Local strict MOTIS installation/configuration is missing. Build it with: sh scripts/build-motis.sh' >&2
            exit 1
        fi
        cd "$DATA"
        if [ "$1" = import ]; then
            exec "$MOTIS" import
        fi
        exec "$MOTIS" server
        ;;
    app)
        if [ ! -x "$ROOT/target/release/dispatch-sat" ]; then
            echo 'Build the dispatcher first: cargo build --release --locked' >&2
            exit 1
        fi
        if [ ! -f "$ROOT/front/dist/index.html" ]; then
            echo 'Build the frontend first: npm --prefix front ci && npm --prefix front run build' >&2
            exit 1
        fi
        cd "$ROOT"
        export DISPATCH_MOTIS_URL="${DISPATCH_MOTIS_URL:-http://127.0.0.1:8081}"
        if [ -z "${DISPATCH_ROAD_BACKEND:-}" ]; then
            if [ -n "${DISPATCH_VALHALLA_URL:-}" ]; then
                DISPATCH_ROAD_BACKEND=valhalla
            else
                DISPATCH_ROAD_BACKEND=motis
            fi
        fi
        export DISPATCH_ROAD_BACKEND
        exec "$ROOT/target/release/dispatch-sat" serve "${2:-8080}"
        ;;
    *)
        echo 'Usage: sh scripts/city.sh [router | import | app [PORT]]' >&2
        exit 2
        ;;
esac
