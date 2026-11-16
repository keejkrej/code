#!/usr/bin/env bash
# Times a real launch: milliseconds from the process starting to its first frame being ready,
# as Rune reports it (RUNE_READY_DIR). Opens the window briefly, once per run, and keeps the
# runs' settings apart from the real ones.
#
#   scripts/launch-time.sh [FOLDER] [RUNS]        # after: cargo build --release
set -euo pipefail
folder="${1:-}"
runs="${2:-5}"
exe="$(dirname "$0")/../target/release/code"
[ -x "$exe.exe" ] && exe="$exe.exe"
work="$(mktemp -d)"
export RUNE_READY_DIR="$work/ready" RUNE_PREFS_DIR="$work/prefs"
[ -n "${OS:-}" ] && command -v cygpath >/dev/null && RUNE_READY_DIR="$(cygpath -m "$RUNE_READY_DIR")" && RUNE_PREFS_DIR="$(cygpath -m "$RUNE_PREFS_DIR")"
mkdir -p "$work/ready" "$work/prefs"
times=()
for _ in $(seq "$runs"); do
  rm -f "$work/ready"/*
  if [ -n "$folder" ]; then "$exe" "$folder" & else "$exe" & fi
  pid=$!
  for _ in $(seq 100); do
    ls "$work/ready"/* >/dev/null 2>&1 && break
    sleep 0.1
  done
  ms="$(sed -n 's/.*"ready_ms":\([0-9.]*\).*/\1/p' "$work/ready"/* 2>/dev/null | head -1)"
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  times+=("${ms:-?}")
  sleep 0.3
done
rm -rf "$work"
echo "ready after (ms): ${times[*]}"
