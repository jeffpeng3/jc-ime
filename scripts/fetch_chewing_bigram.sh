#!/usr/bin/env bash
# 下載新酷音 chewing_v4 語言模型並轉成 0c-compile 的 bigram 格式。
#
# 來源：https://codeberg.org/chewing/libchewing-data（commit 固定，CC-BY-4.0）
# 產出：<out>/chewing-v4.tsv，三欄「前詞<TAB>詞<TAB>P(詞|前詞)」。
# 用法：scripts/fetch_chewing_bigram.sh [輸出目錄]
set -euo pipefail

COMMIT=e5d726d9c09afd562309b05d9adad29940de5ebb
BASE="https://codeberg.org/chewing/libchewing-data/raw/commit/${COMMIT}/dict/chewing_v4"
OUT_DIR=${1:-data/lm/bigram}
CACHE_DIR=${CHEWING_CACHE:-/tmp/opencode/chewing}

# 固定 commit 的檔案雜湊，下載後必對。
BIGRAM_SHA256=ce25ad7e282cfc710f61eea20ba666b3116ffc37de02f92e2df889fbeb811ee5

mkdir -p "$CACHE_DIR" "$OUT_DIR"

fetch() {
  local name=$1
  if [[ ! -f "$CACHE_DIR/$name" ]]; then
    curl -fL --retry 3 -o "$CACHE_DIR/$name" "$BASE/$name"
  fi
}

check_sha() {
  local file=$1 want=$2 got
  got=$(sha256sum "$file" | cut -d' ' -f1)
  if [[ "$got" != "$want" ]]; then
    echo "sha256 不符：$file 期望 $want，實際 $got" >&2
    exit 1
  fi
}

fetch bigram_p50.arpa
check_sha "$CACHE_DIR/bigram_p50.arpa" "$BIGRAM_SHA256"

# ARPA 的 2-gram 是 log10 P(詞|前詞)，換算回機率。
uv run --no-project python - "$CACHE_DIR/bigram_p50.arpa" "$OUT_DIR/chewing-v4.tsv" <<'PY'
import math
import sys

src, dst = sys.argv[1], sys.argv[2]
in_bigram = False
rows = 0
with open(src, encoding="utf-8") as f, open(dst, "w", encoding="utf-8") as out:
    out.write("# 前詞\t詞\tP(詞|前詞)\n")
    out.write("# 來源：libchewing-data dict/chewing_v4/bigram_p50.arpa（CC-BY-4.0），"
              "ARPA 的 log10 機率換算回機率。\n")
    for line in f:
        line = line.rstrip("\n")
        if line.startswith("\\"):
            in_bigram = line.startswith("\\2-grams:")
            continue
        if not in_bigram or not line:
            continue
        parts = line.split(" ")
        if len(parts) != 3:
            raise SystemExit(f"非預期的 2-gram 行：{line!r}")
        log10p, first, second = parts
        p = 10.0 ** float(log10p)
        p = min(max(p, 1e-300), 1.0)
        out.write(f"{first}\t{second}\t{p:.10g}\n")
        rows += 1
print(f"寫入 {rows} 筆 bigram：{dst}", file=sys.stderr)
PY

sha256sum "$OUT_DIR/chewing-v4.tsv"
