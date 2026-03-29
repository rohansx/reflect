#!/usr/bin/env bash
# Record asciinema demo and convert to GIF
set -e
cd "$(dirname "$0")/.."

rm -f /tmp/reflect-demo.db demo/demo.cast demo/demo.gif

echo "Recording asciinema cast..."
asciinema rec demo/demo.cast --command "REFLECT_DB=/tmp/reflect-demo.db python3 demo/demo_client.py" --title "reflect — Self-correction engine for AI agents" --cols 80 --rows 30 --overwrite

echo "Converting to GIF..."
agg demo/demo.cast demo/demo.gif --theme monokai --font-size 16 --speed 1.5

echo "Done! Files:"
ls -lh demo/demo.cast demo/demo.gif
