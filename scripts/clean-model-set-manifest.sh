#!/usr/bin/env bash
# 清掉 models/manifest.json 裡 drop-punct 已移除的角色（zh-punct、ja-punct）。
#
# 用法：
#   scripts/clean-model-set-manifest.sh [資料根目錄]
#
# 資料根目錄省略時讀 JC_DATA_ROOT，預設 /mnt/work/jc/jc-ime-data。
# 只改 <root>/models/manifest.json 的 models 物件，其餘欄位照舊；重複執行
# 不會再動檔案。解析器已明確忽略這兩個舊角色，不清也能啟動，但清除後
# supervisor.log 與資料 manifest 才會一致。
set -euo pipefail

export LC_ALL=C

root=${1:-${JC_DATA_ROOT:-/mnt/work/jc/jc-ime-data}}
manifest=$root/models/manifest.json

if [[ ! -f $manifest ]]; then
    echo "集合 manifest 不存在：$manifest" >&2
    exit 2
fi
if ! command -v python3 >/dev/null 2>&1; then
    echo "找不到 python3" >&2
    exit 2
fi

python3 - "$manifest" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
try:
    data = json.loads(path.read_text(encoding="utf-8"))
except (OSError, json.JSONDecodeError) as error:
    raise SystemExit(f"讀取 {path} 失敗：{error}")

models = data.get("models")
if not isinstance(models, dict):
    raise SystemExit(f"{path} 沒有 models 物件")

removed = [name for name in ("zh-punct", "ja-punct") if name in models]
if not removed:
    print(f"沒有殘留角色：{path}")
    raise SystemExit(0)

for name in removed:
    del models[name]
path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
print(f"已移除 {', '.join(removed)}：{path}")
PY

echo "接著重新產生資料根目錄 manifest.json、打包上傳並更新 variable。"
