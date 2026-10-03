#!/usr/bin/env bash
# The three screens the definition of done asks for (DESIGN.md §11), written to
# docs/screenshots/. Needs the local rig for the non-fake ones: ./dev/local-rig.sh up
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/docs/screenshots"
mkdir -p "$OUT"
cd "$ROOT"

shot() {
  local name="$1" cols="$2" rows="$3" seconds="$4" keys="${5:-}"
  local cmd="${SHOT_CMD:-FAKE=1 cargo run --quiet}"
  python3 dev/shot.py "$cmd" "$cols" "$rows" "$seconds" "$keys" >"$OUT/$name.txt"
  echo "wrote $OUT/$name.txt"
}

shot 120x36-nodes  120 36 5
shot 100x30-nodes  100 30 5
shot 120x36-pivot   120 36 5 "u"
shot 120x36-queue   120 36 5 "2"
