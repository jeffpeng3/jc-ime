#!/usr/bin/env bash
# 在 Linux 上做 jc-ime.wxs 的有限驗證。
#
# WiX v5 官方只支援 Windows，Linux 上會遇到兩個工具本身的限制：
#   1. `Directory/@Name` 的正規化使用 `C:\` 當根目錄，Linux 上判定所有名稱
#      都不是相對路徑（WIX0389）。這裡在暫存副本把名稱換成 "." 繞過。
#   2. 綁定階段要載入 Windows 的 msi.dll，Linux 沒有，`wix build` 只能做到
#      編譯與符號解析（WIX0001 是預期結果）。
#
# 因此這支腳本證明的是「XML、元素、屬性、符號引用、自訂動作排程都能通過
# WiX 編譯器」，不是「MSI 已建好」。真正的 `wix build` 要在 Windows 執行。
#
# 用法：
#   installer/validate-linux.sh [payload 目錄]
#
# payload 省略時，腳本會依 WXS 的 File/@Source 建一個只有 1 位元組內容的暫存
# 目錄，只為了讓檔案存在檢查通過。

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
wxs="$repo_root/installer/jc-ime.wxs"

# 引擎 autostart 必須用 --supervise，且不得用 --socket 覆寫 pipe 名稱。
# 直接伺服模式沒給 socket 參數時會走 stdio，完全不開 pipe，TIP 連不上；
# 只有監督模式才會呼叫 default_supervisor_socket() → resolve_pipe_name()
# 推出 jc-ime-<rid>，與 TIP 一致。顯式 --socket 會讓兩邊名稱不同。
autostart=$(sed -n '/<Component Id="EngineAutostart"/,/<\/Component>/p' "$wxs")
if grep -q -- '--socket' <<<"$autostart"; then
    echo "EngineAutostart 不得帶 --socket；pipe 名稱要交給 resolve_pipe_name() 解析" >&2
    exit 1
fi
if ! grep -q -- '--supervise' <<<"$autostart"; then
    echo "EngineAutostart 必須帶 --supervise；直接伺服模式沒給 socket 參數會退回 stdio，不開 pipe" >&2
    exit 1
fi
if ! grep -q -- '--data' <<<"$autostart" || ! grep -q -- '--settings' <<<"$autostart"; then
    echo "EngineAutostart 必須保留 --data 與 --settings" >&2
    exit 1
fi

if command -v wix >/dev/null 2>&1; then
    wix=wix
elif [ -x "$HOME/.dotnet/tools/wix" ]; then
    wix="$HOME/.dotnet/tools/wix"
else
    echo "找不到 wix；請執行 dotnet tool install --global wix --version 5.*" >&2
    exit 2
fi

payload=${1:-}
cleanup_payload=0
if [ -z "$payload" ]; then
    payload=$(mktemp -d)
    cleanup_payload=1
    while IFS= read -r relative; do
        relative=${relative#'$(var.PayloadDir)'}
        relative=${relative#/}
        mkdir -p "$payload/$(dirname "$relative")"
        printf 'x' > "$payload/$relative"
    done < <(grep -o '\$(var\.PayloadDir)/[^"]*' "$wxs" | sort -u)
fi

lint=$(mktemp --suffix=.wxs)
out_msi=$(mktemp --suffix=.msi)
trap 'rm -f "$lint" "$out_msi"; if [ "$cleanup_payload" = 1 ]; then rm -rf "$payload"; fi' EXIT

# 只把 Directory 元素的 Name 換成 "."；RegistryValue、Shortcut 的 Name 不動。
sed -E 's/(<Directory [^>]*[[:space:]])Name="[^"]*"/\1Name="."/' "$wxs" > "$lint"

version=$(sed -nE 's/^version = "([^"]+)"/\1/p' "$repo_root/Cargo.toml" | head -1)
output=$("$wix" build -arch x64 -d "Version=$version" -d "PayloadDir=$payload" \
    -o "$out_msi" "$lint" 2>&1 || true)

if echo "$output" | grep -E 'error WIX[0-9]+' | grep -vq 'WIX0001'; then
    echo "$output" | grep -E 'error WIX[0-9]+' >&2
    echo "WiX 編譯失敗（上面的錯誤不是 Linux 的已知限制）" >&2
    exit 1
fi

if ! echo "$output" | grep -q 'msi.dll'; then
    echo "$output" >&2
    echo "預期在綁定階段因 msi.dll 失敗，實際沒有；請檢查上面的輸出" >&2
    exit 1
fi

echo "WiX 編譯與符號解析通過（版本 $version）"
echo "綁定階段因 Linux 沒有 msi.dll 停止，這是預期結果；完整建置請在 Windows 執行。"

if [ "$cleanup_payload" = 1 ]; then
    echo "payload 是腳本建立的暫存檔，跳過 check-payload.py；"
    echo "要一起驗資料時，傳入真正暫存的 payload 目錄。"
elif command -v uv >/dev/null 2>&1; then
    uv run --no-project "$repo_root/installer/check-payload.py" "$payload"
else
    echo "找不到 uv；跳過 check-payload.py"
fi
