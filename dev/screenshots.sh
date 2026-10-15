#!/usr/bin/env bash
# The screens of docs/redesign from the fake fleet, written to docs/screenshots/ as text and PNG:
# the dashboard at 120×36, 80×24 and 160×48, every panel zoomed, and the overlays.
#
#   ./dev/screenshots.sh        render through ratatui's TestBackend (exact, colour), then
#                               PNG through a headless Chromium when one is found
#   ./dev/screenshots.sh pty    the real binary in a pseudo-terminal (dev/shot.py), text only;
#                               SHOT_CMD overrides the command, e.g. against the local rig
#
# The TestBackend path is the reference: it is the same frame the terminal gets, with four
# minutes of fake history behind every sparkline. The pty path is for checking the binary
# itself end to end (keys, timers, a real cluster).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/docs/screenshots"
mkdir -p "$OUT"
cd "$ROOT"

if [[ "${1:-}" == "pty" ]]; then
  shot() {
    local name="$1" cols="$2" rows="$3" seconds="$4" keys="${5:-}"
    local cmd="${SHOT_CMD:-FAKE=1 cargo run --quiet --release}"
    python3 dev/shot.py "$cmd" "$cols" "$rows" "$seconds" "$keys" >"$OUT/pty-$name.txt"
    echo "wrote $OUT/pty-$name.txt"
  }
  shot 120x36-dash 120 36 8
  shot 120x36-zoom-queue 120 36 8 $'2\r'
  shot 80x24-dash 80 24 8
  shot 160x48-dash 160 48 8
  exit 0
fi

SHOT_DIR="$OUT" cargo test --quiet ui2::tests::export_screens -- --ignored >/dev/null
echo "wrote $(ls "$OUT"/*.txt | wc -l) text screens to $OUT"

chrome="${CHROME:-}"
if [[ -z "$chrome" ]]; then
  for candidate in /opt/pw-browsers/chromium_headless_shell-*/chrome-linux/headless_shell \
                   "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
                   "$(command -v chromium 2>/dev/null || true)" \
                   "$(command -v google-chrome 2>/dev/null || true)"; do
    if [[ -n "$candidate" && -x "$candidate" ]]; then chrome="$candidate"; break; fi
  done
fi
if [[ -z "$chrome" ]]; then
  echo "no headless Chromium found (set CHROME=…): HTML left in $OUT, no PNGs"
  exit 0
fi

# PNGs for the screens the README shows; the rest stay text.
PNGS="${SHOT_PNGS:-120x36-dash 80x24-dash 160x48-dash 120x36-zoom-fleet 120x36-zoom-queue 120x36-zoom-nodes 120x36-zoom-node 120x36-zoom-airflow 120x36-zoom-jira 120x36-zoom-tape 120x36-zoom-sessions 120x36-zoom-sessions-list 120x36-palette 120x36-explain-node 120x36-cancel-dialog 120x36-float-session 120x36-wizard 120x36-tour 120x36-local 120x36-light}"
for html in "$OUT"/*.html; do
  name=$(basename "$html" .html)
  if [[ " $PNGS " != *" $name "* ]]; then
    rm -f "$html"
    continue
  fi
  dims=${name%%-*}
  cols=${dims%x*}; rows=${dims#*x}
  # DejaVu Sans Mono at 15px: 9.04px a column, 18px a row, plus the page padding.
  w=$(( cols * 904 / 100 + 34 ))
  h=$(( rows * 18 + 30 ))
  "$chrome" --headless --no-sandbox --disable-gpu --hide-scrollbars \
    --force-device-scale-factor="${SHOT_SCALE:-1}" --window-size="$w,$h" \
    --screenshot="$OUT/$name.png" "file://$html" >/dev/null 2>&1
  rm -f "$html"
  # A terminal has few colours: a 256-colour palette is lossless to the eye and a third of
  # the size. Skipped quietly without Pillow.
  python3 - "$OUT/$name.png" <<'PY' 2>/dev/null || true
import sys
from PIL import Image
path = sys.argv[1]
Image.open(path).convert("RGB").quantize(colors=256, method=Image.Quantize.FASTOCTREE, dither=Image.Dither.NONE).save(path, optimize=True)
PY
  echo "wrote $OUT/$name.png"
done
