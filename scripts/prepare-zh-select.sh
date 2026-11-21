#!/usr/bin/env bash
# 把 Funmula 的 zh-select ONNX 轉成隨 MSI 散布的 candle Q8_0 GGUF。
#
# 用法：
#   scripts/prepare-zh-select.sh [資料根目錄]
#
# 資料根目錄省略時讀 JC_DATA_ROOT，預設 /mnt/work/jc/jc-ime-data。
# 這支腳本由 orchestrator 在打包 HF 資料包前執行；CI 只下載打包好的資料包。
#
# 處理順序：
#   1. 缺 model.onnx 時從固定 revision 抓，並驗 SHA-256。
#   2. 用 candle-zh-check --convert --quant q8_0 轉出 model.q8_0.gguf。
#   3. 刪掉 ONNX 與中間的 model.safetensors，只留 GGUF。
#   4. 把 models/manifest.json 的 zh-select model 指向 GGUF，並移除已廢棄的
#      zh-punct、ja-punct 角色（drop-punct 的殘留）。
#   5. 重新產生資料根目錄的 manifest.json，讓 GGUF 與 vocab.json 成為 shipped 檔。
#
# 環境：
#   JC_CLI_DIR   已有 jc-cli 與 candle-zh-check 執行檔的目錄；沒給時用 cargo run --release 編譯。
#   ONNX_URL  覆寫 ONNX 來源；沒給時用下列固定 revision。
set -euo pipefail

export LC_ALL=C

root=${1:-${JC_DATA_ROOT:-/mnt/work/jc/jc-ime-data}}
model_dir="$root/models/zh-select"
onnx="$model_dir/model.onnx"
gguf="$model_dir/model.q8_0.gguf"
safetensors="$model_dir/model.safetensors"
revision=3533e34ef25896afeacc928d136ba7f35783bba7
onnx_sha=4a80caf18acbaddde10c908dcbf35919908d616e94618cdde71fc3409c969261
onnx_url=${ONNX_URL:-https://huggingface.co/Funmula/0c-sequence-d512/resolve/$revision/model.onnx}

if [[ ! -d $root ]]; then
    echo "資料根目錄不存在：$root" >&2
    exit 2
fi
mkdir -p "$model_dir"

if [[ ! -f $onnx ]]; then
    echo "下載 ONNX：$onnx_url"
    curl -fL --retry 3 -o "$onnx" "$onnx_url"
fi
actual=$(sha256sum "$onnx" | cut -d' ' -f1)
if [[ $actual != "$onnx_sha" ]]; then
    echo "ONNX SHA-256 不符：預期 $onnx_sha，實際 $actual" >&2
    exit 1
fi

echo "轉換 ONNX → Q8_0 GGUF"
# 先移除舊檔，避免 GGUF 是符號連結時把內容寫回來源。
rm -f "$gguf"
if [[ -n ${JC_CLI_DIR:-} ]]; then
    converter=("$JC_CLI_DIR/candle-zh-check")
    cli=("$JC_CLI_DIR/jc-cli")
else
    converter=(cargo run --release -p jc-cli --bin candle-zh-check --)
    cli=(cargo run --release -p jc-cli --)
fi
"${converter[@]}" --convert --quant q8_0 --model-dir "$model_dir"

if [[ ! -s $gguf ]]; then
    echo "找不到轉好的 GGUF：$gguf" >&2
    exit 1
fi
# ONNX 是轉檔來源，safetensors 是 GGUF 的中間產物；兩者都不進資料包。
rm -f "$onnx" "$safetensors"

python3 - "$root" <<'PY'
import json
import sys
from pathlib import Path

root = Path(sys.argv[1])
path = root / "models" / "manifest.json"
data = json.loads(path.read_text(encoding="utf-8"))
models = data.get("models", {})
spec = models.get("zh-select")
if spec is not None:
    spec["model"] = "model.q8_0.gguf"
    spec["vocab"] = "vocab.json"
# drop-punct 已移除這兩個角色；順手清掉舊資料目錄的殘留鍵。
removed = [name for name in ("zh-punct", "ja-punct") if name in models]
for name in removed:
    del models[name]
path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
if removed:
    print(f"已從集合 manifest 移除角色：{', '.join(removed)}")
PY

"${cli[@]}" data manifest "$root" --out "$root/manifest.json"

gguf_sha=$(sha256sum "$gguf" | cut -d' ' -f1)
gguf_bytes=$(stat -Lc %s "$gguf")
echo "完成：$gguf（$gguf_bytes bytes）"
echo "SHA-256：$gguf_sha"
echo "接著執行 scripts/data-bundle.sh 並上傳新的資料包。"
