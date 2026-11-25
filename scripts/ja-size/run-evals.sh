#!/usr/bin/env bash
# 重跑 ja-size 報告裡的所有日文評測。
#
# 需求：已建好的 Linux jc-cli（`cargo build --release -p jc-cli`）、
# `uv`、題庫在 0c-testdata，pack 在資料根目錄。
#
# 用法：
#   scripts/ja-size/run-evals.sh <jc-cli> <data-root> <out-dir>
#
# 產出：<out-dir>/{base,trim300k,trim500k,conn-global-u8,conn-row-u8}-<corpus>.json
# 另外把 Q4_K_M 的 Jinen 換模型評測寫成 jinen-q4-<corpus>.json；Q5 是 base 的
# neural 版本，需先下載 Q4_K_M：
#   curl -L -o /tmp/jinen-q4.gguf \
#     https://huggingface.co/togatogah/jinen-v2-small.gguf/resolve/main/jinen-v2-small-Q4_K_M.gguf
set -euo pipefail

JC=${1:?用法：run-evals.sh <jc-cli> <data-root> <out-dir>}
ROOT=${2:?缺少資料根目錄}
OUT=${3:?缺少輸出目錄}
CORPORA=${CORPORA:-"modern-500 anthy-corpus1 ajimee-200 hard-cases"}
PACK="$ROOT/packs/ja-lexicon-mozc-v2.pack"
HERE=$(cd "$(dirname "$0")" && pwd)

mkdir -p "$OUT" /tmp/opencode/ja-size-variants
V=/tmp/opencode/ja-size-variants

echo "== 產生變體 pack"
uv run --with zstandard "$HERE/analyze_pack.py" "$PACK" \
    --write-trim "$V/trim-300k.pack" --keep 300000 >/dev/null
uv run --with zstandard "$HERE/analyze_pack.py" "$PACK" \
    --write-trim "$V/trim-500k.pack" --keep 500000 >/dev/null
python3 - "$HERE" "$PACK" "$V" <<'PY'
import sys
sys.path.insert(0, sys.argv[1])
import analyze_pack as A

pack = A.load_pack(sys.argv[2])
entries = list(pack.entries())
matrix = pack.matrix()
for scheme in ("global-u8", "row-u8"):
    quantized, _ = (
        A.quant_global_u8(matrix) if scheme == "global-u8" else A.quant_row_u8(matrix, pack.context_count)
    )
    data = A.encode_pack(pack, entries, quantized, 0)
    open(f"{sys.argv[3]}/conn-{scheme}.pack", "wb").write(data)
    print(f"  {scheme}: {len(data)} bytes")
PY

run() {
    local name=$1 pack=$2 corpus=$3
    "$JC" eval ja --corpus "$corpus" ${pack:+--pack "$pack"} --neural off \
        --json --items >"$OUT/$name-$corpus.json"
}

echo "== 統計路徑（--neural off）"
for corpus in $CORPORA; do
    run base "" "$corpus"
    run trim300k "$V/trim-300k.pack" "$corpus"
    run trim500k "$V/trim-500k.pack" "$corpus"
    run conn-global-u8 "$V/conn-global-u8.pack" "$corpus"
    run conn-row-u8 "$V/conn-row-u8.pack" "$corpus"
done

echo "== Jinen（--neural quality）"
Q4=${Q4_GGUF:-/tmp/opencode/jinen-q4/jinen-v2-small-Q4_K_M.gguf}
if [[ -f $Q4 ]]; then
    for corpus in $CORPORA; do
        JC_JINEN_GGUF=$Q4 "$JC" eval ja --corpus "$corpus" --neural quality \
            --json --items >"$OUT/jinen-q4-$corpus.json"
    done
else
    echo "略過 Jinen Q4：找不到 $Q4" >&2
fi

echo "== 逐題差異"
python3 - "$OUT" "$CORPORA" <<'PY'
import json, sys

out, corpora = sys.argv[1], sys.argv[2].split()
for corpus in corpora:
    try:
        base = {i["id"]: i for i in json.load(open(f"{out}/base-{corpus}.json"))["corpora"][0]["items"]}
    except FileNotFoundError:
        continue
    for variant in ("trim300k", "trim500k", "conn-global-u8", "conn-row-u8"):
        try:
            other = {i["id"]: i for i in json.load(open(f"{out}/{variant}-{corpus}.json"))["corpora"][0]["items"]}
        except FileNotFoundError:
            continue
        regress = sum(1 for k in base if base[k]["top1"] and not other[k]["top1"])
        improve = sum(1 for k in base if not base[k]["top1"] and other[k]["top1"])
        top1 = sum(i["top1"] for i in other.values()) / len(other)
        print(f"{corpus:14} {variant:14} top1={top1*100:5.1f}% regress={regress:2} improve={improve:2}")
PY
