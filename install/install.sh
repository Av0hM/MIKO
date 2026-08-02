#!/bin/bash
set -e
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
mkdir -p ~/.config/samos
mkdir -p ~/.config/eww
rsync -a --delete "$ROOT/eww/" ~/.config/eww/
rsync -a --delete "$ROOT/runtime/" ~/.config/samos/
echo "SamOS installed successfully."
