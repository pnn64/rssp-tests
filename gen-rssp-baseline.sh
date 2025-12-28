#!/usr/bin/env bash
set -euo pipefail

# Config (override via env if you want)
PACKS_DIR="${PACKS_DIR:-packs}"
BASELINE_DIR="${BASELINE_DIR:-baseline}"
ZSTD_LEVEL="${ZSTD_LEVEL:-10}"
ZSTD_THREADS="${ZSTD_THREADS:-0}"   # 0 = all cores

# rssp binary (override via env)
RSSP_BIN="${RSSP_BIN:-rssp}"

FORCE=0
if [[ "${1:-}" == "--force" ]]; then
  FORCE=1
  shift
fi

die() { echo "ERROR: $*" >&2; exit 1; }

# Resolve RSSP_BIN:
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
if [[ "$RSSP_BIN" != */* ]]; then
  if [[ -x "$SCRIPT_DIR/target/release/$RSSP_BIN" ]]; then
    RSSP_BIN="$SCRIPT_DIR/target/release/$RSSP_BIN"
  else
    RSSP_BIN="$(command -v "$RSSP_BIN" || true)"
  fi
fi

# --- sanity checks ---
command -v zstd   >/dev/null 2>&1 || die "zstd not found in PATH"
command -v md5sum >/dev/null 2>&1 || die "md5sum not found in PATH"
[[ -n "$RSSP_BIN" && -x "$RSSP_BIN" ]] || die "rssp not found/executable. Set RSSP_BIN or ensure it’s in PATH."

# Temp workspace for decompressing *.zst inputs
TMPDIR="$(mktemp -d)"
cleanup() { rm -rf "$TMPDIR"; }
trap cleanup EXIT INT TERM

# Find simfiles (handles spaces/quotes/etc safely)
find "$PACKS_DIR" -type f \
  \( -name '*.sm' -o -name '*.ssc' -o -name '*.sm.zst' -o -name '*.ssc.zst' \) \
  -print0 |
while IFS= read -r -d '' f; do
  src="$f"
  tmpfile=""

  # If compressed, decompress to a temp file (keeping .sm/.ssc at the end)
  if [[ "$f" == *.zst ]]; then
    base="$(basename "${f%.zst}")"          # e.g. "Song.sm"
    ext="${base##*.}"                       # sm or ssc

    tmpbase="$(mktemp -p "$TMPDIR" "rssp.XXXXXX")"
    tmpfile="${tmpbase}.${ext}"             # .../rssp.ABC123.sm (or .ssc)
    mv -- "$tmpbase" "$tmpfile"

    if ! zstd -q -dc -- "$f" >"$tmpfile"; then
      echo "WARN: failed to decompress: $f" >&2
      rm -f -- "$tmpfile"
      continue
    fi
    src="$tmpfile"
  fi

  # MD5 of the *uncompressed* .sm/.ssc bytes
  md5="$(md5sum -- "$src" | awk '{print $1}')"
  prefix="${md5:0:2}"
  outdir="$BASELINE_DIR/$prefix"
  outfile="$outdir/$md5.rssp.json.zst"

  if [[ -f "$outfile" && "$FORCE" -eq 0 ]]; then
    echo "skip: $outfile"
    [[ -n "$tmpfile" ]] && rm -f -- "$tmpfile"
    continue
  fi

  mkdir -p "$outdir"

  echo "gen : $outfile"
  echo "      from: $f"

  # Run rssp -> compress JSON with zstd
  if ! "$RSSP_BIN" "$src" --json \
      | zstd -q -"${ZSTD_LEVEL}" -T"${ZSTD_THREADS}" -f -o "$outfile"; then
    echo "WARN: rssp failed for: $f" >&2
    rm -f -- "$outfile"
  fi

  [[ -n "$tmpfile" ]] && rm -f -- "$tmpfile"
done
