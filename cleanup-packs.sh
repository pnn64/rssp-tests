#!/usr/bin/env bash
set -euo pipefail

# Config (override via env)
PACKS_DIR="${PACKS_DIR:-packs}"
ZSTD_LEVEL="${ZSTD_LEVEL:-10}"
ZSTD_THREADS="${ZSTD_THREADS:-0}"   # 0 = all cores

# --- helpers ---
die() { echo "ERROR: $*" >&2; exit 1; }

# --- sanity checks ---
command -v zstd >/dev/null 2>&1 || die "zstd not found in PATH"
[[ -d "$PACKS_DIR" ]] || die "PACKS_DIR not found or not a directory: $PACKS_DIR"

# 1) Delete everything except: .sm, .ssc, .dwi, and their .zst variants
echo "Cleaning: deleting files not matching {*.sm, *.ssc, *.dwi, *.sm.zst, *.ssc.zst, *.dwi.zst}"

# Note: keep the list explicit to avoid subtle suffix mistakes.
find "$PACKS_DIR" -type f \
  ! \( -name '*.sm' -o -name '*.ssc' -o -name '*.dwi' -o -name '*.sm.zst' -o -name '*.ssc.zst' -o -name '*.dwi.zst' \) \
  -print0 |
while IFS= read -r -d '' f; do
  echo "del : $f"
  rm -f -- "$f"
done

# 2) Compress plain simfiles to .zst (level N), then delete the original
echo "Compressing: *.sm/*.ssc/*.dwi -> *.zst (level $ZSTD_LEVEL)"

find "$PACKS_DIR" -type f \( -name '*.sm' -o -name '*.ssc' -o -name '*.dwi' \) -print0 |
while IFS= read -r -d '' f; do
  out="$f.zst"

  # If already compressed, don't redo work (and don't delete the original)
  if [[ -f "$out" ]]; then
    echo "skip: $out"
    continue
  fi

  echo "zst : $out"
  if zstd -q -"${ZSTD_LEVEL}" -T"${ZSTD_THREADS}" --rm -o "$out" -- "$f"; then
    :
  else
    echo "WARN: failed to compress: $f" >&2
    rm -f -- "$out"
  fi
done
