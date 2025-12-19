#!/usr/bin/env bash
set -euo pipefail

# Config (override via env)
PACKS_DIR="${PACKS_DIR:-tests/data/packs}"
BASELINE_DIR="${BASELINE_DIR:-tests/data/baseline}"
ZSTD_LEVEL="${ZSTD_LEVEL:-10}"
ZSTD_THREADS="${ZSTD_THREADS:-0}"   # 0 = all cores

# itgmania-reference-harness binary (override via env)
HARNESS_BIN="${HARNESS_BIN:-./build/itgmania-reference-harness}"

FORCE=0
if [[ "${1:-}" == "--force" ]]; then
  FORCE=1
  shift
fi

# --- sanity checks ---
command -v zstd   >/dev/null 2>&1 || { echo "ERROR: zstd not found in PATH" >&2; exit 1; }
command -v md5sum >/dev/null 2>&1 || { echo "ERROR: md5sum not found in PATH" >&2; exit 1; }

if [[ ! -x "$HARNESS_BIN" ]]; then
  echo "ERROR: itgmania-reference-harness not executable at: $HARNESS_BIN" >&2
  echo "       Set HARNESS_BIN (e.g. HARNESS_BIN=./build/itgmania-reference-harness)" >&2
  exit 1
fi

# Track any temporary uncompressed files we create (strip .zst) so we can clean up on exit.
CREATED_PLAINS=()
cleanup() {
  for p in "${CREATED_PLAINS[@]}"; do
    rm -f -- "$p"
  done
}
trap cleanup EXIT INT TERM

# Use process substitution to avoid the while-loop-in-a-subshell gotcha
while IFS= read -r -d '' f; do
  src="$f"
  created_plain=""

  # Determine MD5 of *uncompressed* bytes, and decide what path to run harness on.
  if [[ "$f" == *.zst ]]; then
    plain="${f%.zst}"  # stable path: ".../satellite.sm" from ".../satellite.sm.zst"

    if [[ -f "$plain" ]]; then
      # Uncompressed already exists; use it directly.
      md5="$(md5sum -- "$plain" | awk '{print $1}')"
      src="$plain"
    else
      # Compute md5 by streaming decompression (no temp file yet)
      md5="$(zstd -q -dc -- "$f" | md5sum | awk '{print $1}')"
      src="$plain"  # if we need to run, we'll materialize it at this stable path
    fi
  else
    md5="$(md5sum -- "$f" | awk '{print $1}')"
  fi

  prefix="${md5:0:2}"
  outdir="$BASELINE_DIR/$prefix"
  outfile="$outdir/$md5.json.zst"

  if [[ -f "$outfile" && "$FORCE" -eq 0 ]]; then
    echo "skip: $outfile"
    continue
  fi

  mkdir -p "$outdir"

  # If input was .zst and the stable uncompressed file doesn't exist, create it now.
  if [[ "$f" == *.zst ]]; then
    plain="${f%.zst}"
    if [[ ! -f "$plain" ]]; then
      echo "decomp: $plain"
      if ! zstd -q -dc -- "$f" >"$plain"; then
        echo "WARN: failed to decompress: $f" >&2
        rm -f -- "$plain"
        continue
      fi
      CREATED_PLAINS+=("$plain")
      created_plain="$plain"
    fi
  fi

  echo "gen : $outfile"
  echo "      from: $f"

  # Run harness (file-only CLI) -> compress JSON with zstd level 10
  if ! "$HARNESS_BIN" "$src" \
      | zstd -q -"${ZSTD_LEVEL}" -T"${ZSTD_THREADS}" -f -o "$outfile"; then
    echo "WARN: harness failed for: $f" >&2
    rm -f -- "$outfile"
  fi

  # Remove the uncompressed file if we created it in this run
  if [[ -n "$created_plain" ]]; then
    rm -f -- "$created_plain"
  fi
done < <(
  find "$PACKS_DIR" -type f \
    \( -name '*.sm' -o -name '*.ssc' -o -name '*.sm.zst' -o -name '*.ssc.zst' \) \
    -print0
)
