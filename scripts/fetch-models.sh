#!/usr/bin/env bash
# DEPRECATED thin wrapper — model fetching is now built into the binary:
#
#   docparse fetch-models [ocr|ppocr-v6|layout|unirec|ppv2|all] [--dir DIR]
#
# Pure Rust over the HuggingFace tree API — no `hf` CLI, no Python, no shell
# downloader. Kept for scripts that still call this path.
set -euo pipefail
exec docparse fetch-models "$@"
