#!/usr/bin/env bash
# 在 Linux 上做 Inno Setup 安裝腳本的有限靜態驗證。
#
# 本機沒有 Inno Setup（ISCC 只有 Windows 版），所以這裡證明的是：
#   1. jc.iss 的 [Setup]、[Run]、[Code] 關鍵設定都在；
#   2. jc.iss 的每個 [Files] Source 都對得上 build.ps1 的 staging 或 repo 檔案；
#   3. 有資料根目錄時，manifest 的 shipped 清單與 jc.iss 的 data 條目一致；
#   4. 有 payload 時，check-payload.py 的大小與 SHA-256 檢查也一起跑。
#
# 真正的編譯、簽章與安裝／升級／反安裝必須在 Windows 執行。
#
# 用法：
#   installer/validate-linux.sh [payload 目錄]
#
# 環境變數：
#   JC_DATA_ROOT  資料根目錄；有 manifest.json 時順便比對 shipped 清單。

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
iss="$repo_root/installer/jc.iss"
build="$repo_root/installer/build.ps1"
payload=${1:-}
fail=0

error() {
    echo "錯誤：$*" >&2
    fail=1
}

require_line() {
    if ! grep -Fq -- "$1" "$iss"; then
        error "jc.iss 缺少：$1"
    fi
}

if [ ! -f "$iss" ]; then
    echo "找不到 $iss" >&2
    exit 2
fi
if [ ! -f "$build" ]; then
    echo "找不到 $build" >&2
    exit 2
fi

# [Setup] 與流程的關鍵設定。少任何一項都會回到 MSI 時代的症狀。
require_line 'AppId={{EF7AD1D4-1BA4-41AE-AD3A-768BEA4D64B4}'
require_line 'PrivilegesRequired=admin'
require_line 'ArchitecturesInstallIn64BitMode=x64compatible'
require_line 'DefaultDirName={autopf}\jc-ime'
require_line 'CloseApplications=no'
require_line 'RestartApplications=no'
require_line 'function PrepareToInstall'
require_line 'MoveFileExW@kernel32.dll stdcall'
require_line 'GetDateTimeString('
require_line 'runasoriginaluser'
require_line 'machine-register'
require_line 'user-install'
require_line 'user-cleanup-legacy'
require_line 'machine-unregister'
require_line 'post-install-doctor.ps1'
require_line 'Flags: runhidden waituntilterminated runasoriginaluser'
require_line 'Type: filesandordirs; Name: "{localappdata}\jc-ime\doctor"'
require_line 'MOVEFILE_DELAY_UNTIL_REBOOT'
require_line 'WindowsInstaller'
require_line 'msiexec.exe'

# 引擎 autostart 必須用 --supervise，資料、設定與個人資料路徑都不能少。
# --socket **不能**出現：pipe 名由 jc_windows::resolve_pipe_name() 決定，TIP 與
# 引擎呼叫同一個函式；autostart 自己算 rid 只會多一個出錯點。
if ! grep -Fq -- '--supervise' "$iss"; then
    error 'jc.iss 的引擎 autostart 缺少 --supervise'
fi
if ! grep -Fq -- '--data' "$iss" || ! grep -Fq -- '--settings' "$iss"; then
    error 'jc.iss 的引擎 autostart 缺少 --data 或 --settings'
fi
if ! grep -Fq -- '--personal-dir' "$iss"; then
    error 'jc.iss 的引擎 autostart 缺少 --personal-dir（個人資料不能寫進 ProgramData）'
fi
# 只看 Run 機碼那一行，註解裡提到 --socket 不算。
if sed -n 's/.*ValueName: "jc-ime-engine".*/&/p' "$iss" | grep -Fq -- '--socket'; then
    error 'jc.iss 的引擎 autostart 不該帶 --socket：pipe 名要由 resolve_pipe_name() 決定'
fi

# [Files] 的 Source 清單。Source 一律寫成 {#PayloadDir}\relative。
mapfile -t sources < <(
    sed -n '/^\[Files\]/,/^\[/p' "$iss" |
        sed -n 's/.*Source: "{#PayloadDir}[\\/]\+\([^"]*\)".*/\1/p' |
        sed 's/\\/\//g'
)
if [ "${#sources[@]}" -eq 0 ]; then
    error 'jc.iss 的 [Files] 沒有任何 Source'
fi

for relative in "${sources[@]}"; do
    case "$relative" in
        data/*)
            ;;
        legal/*|LICENSE|NOTICE)
            if [ ! -f "$repo_root/$relative" ]; then
                error "repo 缺少 $relative（jc.iss 引用）"
            fi
            ;;
        *)
            # 程式檔必須是 build.ps1 真的會放進 payload 的名稱。
            if ! grep -Fq -- "'$relative'" "$build"; then
                error "build.ps1 沒有 staging $relative（jc.iss 引用）"
            fi
            ;;
    esac
done

# 有資料根目錄時比對 manifest 的 shipped 清單。data/manifest.json 自己也進 payload。
data_root=${JC_DATA_ROOT:-}
if [ -n "$data_root" ] && [ -f "$data_root/manifest.json" ]; then
    if ! python3 - "$iss" "$data_root/manifest.json" <<'PY'
import json
import re
import sys
from pathlib import Path

iss = Path(sys.argv[1])
manifest = Path(sys.argv[2])
text = iss.read_text(encoding="utf-8-sig")
pattern = re.compile(r'Source:\s*"\{#PayloadDir\}[\\/]+data[\\/]+([^"]+)"')
actual = {"data/" + match.group(1).replace("\\", "/") for match in pattern.finditer(text)}
shipped = {
    "data/" + entry["path"]
    for entry in json.loads(manifest.read_text(encoding="utf-8"))["files"]
    if entry.get("shipped")
}
missing = sorted(shipped - actual)
extra = sorted(actual - shipped - {"data/manifest.json"})
for path in missing:
    print(f"錯誤：manifest 隨附但 jc.iss 沒引用：{path}", file=sys.stderr)
for path in extra:
    print(f"錯誤：jc.iss 引用但 manifest 沒有隨附：{path}", file=sys.stderr)
raise SystemExit(1 if missing or extra else 0)
PY
    then
        fail=1
    fi
else
    echo '略過 manifest 比對（沒有 JC_DATA_ROOT/manifest.json）'
fi

# 有真的 payload 時，讓 check-payload.py 驗檔案存在、大小與 SHA-256。
if [ -n "$payload" ]; then
    if [ ! -d "$payload" ]; then
        error "payload 目錄不存在：$payload"
    elif command -v python3 >/dev/null 2>&1; then
        if ! python3 "$repo_root/installer/check-payload.py" "$payload"; then
            fail=1
        fi
    else
        echo '找不到 python3；跳過 check-payload.py'
    fi
else
    echo '沒有傳 payload；跳過檔案大小與雜湊檢查'
fi

if [ "$fail" -ne 0 ]; then
    echo 'jc.iss 靜態檢查失敗' >&2
    exit 1
fi

echo "jc.iss 靜態檢查通過（$((${#sources[@]})) 個 [Files] 條目）"
echo 'ISCC 編譯、簽章與安裝／升級／反安裝只能在 Windows 驗證。'
