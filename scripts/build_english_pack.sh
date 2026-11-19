#!/bin/sh
# 建置英文詞表資料包：ESDB／SCOWL 正規化 TSV 加產品詞表 → words.bin。
#
# 來源：ref/0c-ime/data/english（esdb.tsv、technical.tsv、source.json、LICENSE.txt）
# 產出：<out>/words.bin、manifest.json、NOTICE.txt；輸出須為新目錄或空目錄。
# 用法：scripts/build_english_pack.sh [來源目錄] [輸出目錄]
set -eu

SOURCE=${1:-/mnt/work/jc/ref/0c-ime/data/english}
OUT=${2:-/mnt/work/jc/jc-ime-data/english}

if [ ! -d "$SOURCE" ]; then
    echo "找不到來源目錄：$SOURCE" >&2
    exit 1
fi

if [ -d "$OUT" ] && [ -n "$(ls -A "$OUT")" ]; then
    echo "輸出目錄不是空的：$OUT" >&2
    echo "請先清空後重跑，或把輸出目錄當第二個參數。" >&2
    exit 1
fi

exec mise exec -- cargo run -q -p jc-englishpack --release -- build --source "$SOURCE" --out "$OUT"
