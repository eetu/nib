#!/usr/bin/env bash
# Regenerate the home-screen / PWA icon set from the source SVGs.
# Requires librsvg:  brew install librsvg
# Rerun after editing favicon.svg or icon-maskable.svg, then commit the PNGs
# (they're committed so the build needs no rasterizer). The source SVGs carry an
# opaque white tile, so no background flag is needed here.
set -euo pipefail
cd "$(dirname "$0")/../static"

rsvg-convert -w 180 -h 180 favicon.svg       -o apple-touch-icon.png
rsvg-convert -w 192 -h 192 favicon.svg       -o icon-192.png
rsvg-convert -w 512 -h 512 favicon.svg       -o icon-512.png
rsvg-convert -w 32  -h 32  favicon.svg       -o favicon-32.png
rsvg-convert -w 192 -h 192 icon-maskable.svg -o icon-192-maskable.png
rsvg-convert -w 512 -h 512 icon-maskable.svg -o icon-512-maskable.png
