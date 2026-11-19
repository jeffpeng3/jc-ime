#!/usr/bin/env bash
# 匯出 candle 用的 GGUF 量化權重，並量檔案大小、RSS、延遲與數值差異。
#
# ```sh
# scripts/candle-quant-run.sh            # 全部重跑
# OUT=/tmp/quant-rep scripts/candle-quant-run.sh
# ```
#
# 每個權重檔都在獨立行程量測：RSS 只有在單一模型載入的乾淨行程裡才有意義，
# 所以這裡一個模型跑一次 `candle-quant bench`，不是在同一行程輪流載入。
# 執行緒固定 8（RAYON_NUM_THREADS），不另外調參。
set -euo pipefail

ROOT=${ROOT:-/mnt/work/jc/jc-ime-data}
MODELS=${MODELS:-$ROOT/models}
DATA=${DATA:-/mnt/work/0c}
OUT=${OUT:-$PWD/bench}
BIN=${BIN:-$PWD/target/release/candle-quant}
THREADS=${THREADS:-8}
RERANK_BENCH=${RERANK_BENCH:-$DATA/data/benchmarks/homophone-extra-v1/test.jsonl}
PUNCT_TOKENS=${PUNCT_TOKENS:-$DATA/data/punctuation_public_core_ja/validation/validation-00000.tokens.i32}

mkdir -p "$OUT"
export RAYON_NUM_THREADS=$THREADS

quant_dir=$MODELS/quant/candle

# 1. 匯出三種等級；原始 safetensors 只讀。
for dtype in q8_0 q5k q4k; do
  "$BIN" export \
    --input "$MODELS/reranker-extra/model.safetensors" \
    --output "$quant_dir/ja-rerank/$dtype/model.gguf" \
    --dtype "$dtype" >"$OUT/export-ja-rerank-$dtype.txt"
  "$BIN" export --punct \
    --input "$MODELS/punctuation-public-core/model.safetensors" \
    --output "$quant_dir/ja-punct/$dtype/model.gguf" \
    --dtype "$dtype" >"$OUT/export-ja-punct-$dtype.txt"
done

# 2. 每個權重檔各開一次行程量 RSS 與延遲。fp32 用 safetensors 當基準。
for dtype in fp32 q8_0 q5k q4k; do
  if [ "$dtype" = fp32 ]; then
    rerank_weights=$MODELS/reranker-extra/model.safetensors
    punct_weights=$MODELS/punctuation-public-core/model.safetensors
  else
    rerank_weights=$quant_dir/ja-rerank/$dtype/model.gguf
    punct_weights=$quant_dir/ja-punct/$dtype/model.gguf
  fi
  "$BIN" bench \
    --model-dir "$MODELS/reranker-extra" \
    --weights "$rerank_weights" \
    --benchmark "$RERANK_BENCH" \
    --groups 200 --iters 100 \
    --out "$OUT/ja-rerank-$dtype.json" >/dev/null
  "$BIN" punct \
    --model-dir "$MODELS/punctuation-public-core" \
    --weights "$punct_weights" \
    --tokens "$PUNCT_TOKENS" \
    --groups 200 --iters 100 \
    --out "$OUT/ja-punct-$dtype.json" >/dev/null
done

# 3. 與 fp32 比數值與排序。
for dtype in q8_0 q5k q4k; do
  "$BIN" compare \
    --baseline "$OUT/ja-rerank-fp32.json" \
    --other "$OUT/ja-rerank-$dtype.json" >"$OUT/compare-ja-rerank-$dtype.json"
  "$BIN" compare \
    --baseline "$OUT/ja-punct-fp32.json" \
    --other "$OUT/ja-punct-$dtype.json" >"$OUT/compare-ja-punct-$dtype.json"
done

echo "結果在 $OUT"
