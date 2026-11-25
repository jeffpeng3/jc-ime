#!/usr/bin/env bash
# shellcheck disable=SC2016  # 說明文字用反引號標記程式碼，不是要 shell 展開。
# 產生 GitHub Release 說明，或檢查說明引用的資料檔是否齊全。
#
# 用法：
#   scripts/release-notes.sh <版本> <iso版本> <true|false> <資料根目錄> <CHANGELOG 路徑>
#   scripts/release-notes.sh --check <資料根目錄>
#
# 一般模式把完整說明寫到 stdout。資料根目錄只需要有 manifest.json，用來讀
# 資料版本；CHANGELOG 取 `## <版本>` 到下一個 `## ` 之間的內容。
#
# --check 模式驗證說明引用的資料檔都存在（bigram、英文詞表、zh-select、
# jinen 模型等），缺少的檔案逐行以 ::error:: 印出並 exit 1。
set -euo pipefail

export LC_ALL=C

usage() {
    sed -n '2,13p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

if [[ ${1:-} == -h || ${1:-} == --help ]]; then
    usage
    exit 0
fi

if ! command -v python3 >/dev/null 2>&1; then
    echo "::error::找不到 python3；scripts/release-notes.sh 需要 python3 讀 manifest.json" >&2
    exit 2
fi

check_files() {
    local root=$1
    local missing=0
    local relative
    local path
    for relative in \
        build-zh/lexicon.bin \
        build-zh/manifest.json \
        build-zh-bigram/lm.bin \
        english/words.bin \
        english/manifest.json \
        english/NOTICE.txt \
        models/zh-select/model.q8_0.gguf \
        models/zh-select/vocab.json \
        models/zh-select/manifest.json \
        models/external/jinen-v2-small/jinen-v2-small-Q5_K_M.gguf
    do
        path="${root}/${relative}"
        if [[ ! -f ${path} ]]; then
            echo "::error::Release 說明引用的資料檔不存在：${relative}（預期路徑：${path}）" >&2
            missing=$((missing + 1))
        fi
    done
    if [[ ! -f "${root}/manifest.json" ]]; then
        echo "::error::資料根目錄缺少根 manifest：${root}/manifest.json" >&2
        missing=$((missing + 1))
    fi
    if (( missing > 0 )); then
        echo "::error::資料檢查失敗：共缺少 ${missing} 個檔案（資料根目錄：${root}）" >&2
        return 1
    fi
    echo "Release 說明引用的資料檔齊全：${root}"
}

if [[ ${1:-} == --check ]]; then
    if [[ $# -ne 2 ]]; then
        usage >&2
        exit 2
    fi
    root=$2
    if [[ ! -d ${root} ]]; then
        echo "::error::資料根目錄不存在：${root}" >&2
        exit 2
    fi
    root=$(cd "${root}" && pwd -P)
    check_files "${root}"
    exit $?
fi

if [[ $# -ne 5 ]]; then
    usage >&2
    exit 2
fi

version=$1
iso_version=$2
signed=$3
data_root=$4
changelog=$5

case ${signed} in
    true | false) ;;
    *)
        echo "::error::簽章參數必須是 true 或 false：${signed}" >&2
        exit 2
        ;;
esac
if [[ ! -d ${data_root} ]]; then
    echo "::error::資料根目錄不存在：${data_root}" >&2
    exit 2
fi
if [[ ! -f "${data_root}/manifest.json" ]]; then
    echo "::error::資料根目錄缺少根 manifest：${data_root}/manifest.json" >&2
    exit 2
fi
if [[ ! -f ${changelog} ]]; then
    echo "::error::CHANGELOG 不存在：${changelog}" >&2
    exit 2
fi

data_version=$(python3 - "${data_root}/manifest.json" <<'PY'
import json
import sys
from pathlib import Path

manifest = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
print(manifest.get("version", "unknown"))
PY
)
date_utc=$(date -u +%Y-%m-%d)

section=$(mktemp)
trap 'rm -f "${section}"' EXIT

awk -v ver="${version}" '
  /^## / {
    if (found) exit
    heading = $0
    sub(/^##[[:space:]]+/, "", heading)
    split(heading, parts, /[[:space:]]+/)
    if (parts[1] == ver) {
      found = 1
      next
    }
  }
  found { print }
' "${changelog}" > "${section}"

if [[ ! -s ${section} ]]; then
    printf '本版沒有 CHANGELOG 條目；請看 commit 記錄。\n' > "${section}"
fi

printf '## 版本\n\n`%s`（%s）\n\n' "${version}" "${date_utc}"
if [[ "${iso_version}" != "${version}" ]]; then
    printf '這是 pre-release。setup.exe 的顯示版本與檔名是 `%s`，\n' "${version}"
    printf '檔案內嵌的 VersionInfoVersion 只有 `%s`（四段數字）。\n\n' "${iso_version}"
fi
printf '## 模型與資料版本\n\n`%s`（安裝包內 `data/manifest.json`，涵蓋詞典與模型）\n\n' "${data_version}"
printf '## 資料包\n\n'
printf -- '- 注音 bigram 語言模型 `build-zh-bigram/lm.bin` 是必要檔案；unigram `build-zh/lm.bin` 已退役，安裝包不含。\n'
printf -- '- 英文詞表 `english/words.bin`（82,015 詞）隨附，不需另外下載。\n'
printf -- '- 注音神經選字模型（Funmula `zh-select`，`model.q8_0.gguf`、`vocab.json`、`manifest.json`）隨附，不需另外下載。\n\n'
printf '## 變更摘要\n\n'
cat "${section}"
printf '\n## 下載\n\n'
printf '| 平台 | 檔案 |\n|---|---|\n'
printf '| Windows x64 | `jc-ime-%s-x64-setup.exe` |\n' "${version}"
printf '| Linux x86_64 | `jc-ime-%s-linux-x86_64.tar.gz` |\n\n' "${version}"
printf '每個檔案旁都附 `.sha256`，下載後請先核對。\n\n'
printf '## 簽章\n\n'
if [[ ${signed} == true ]]; then
    printf 'Windows 執行檔與 setup.exe 已使用專案憑證簽章。\n\n'
else
    printf '**未簽章**：本次沒有簽章憑證。Windows 可能顯示 SmartScreen 警告；\n'
    printf '企業的 AppLocker／WDAC 白名單也會擋下未簽章安裝程式。\n\n'
fi
printf '## 已知限制\n\n'
printf -- '- Windows TIP 同時包含 x64 與 x86 兩份 DLL，共用同一個「jc 注音」項目；32 位元程式（例如舊版 Office）不需要手動切換。\n'
printf -- '- 安裝程式是 per-machine，需要系統管理員；只會把輸入法加進執行安裝的帳號。\n'
printf -- '- Linux 版目前只有引擎、CLI 與設定程式，沒有桌面輸入法整合。\n'
printf -- '- 設定變更後最多 1 秒內熱重載，不需要重新啟動引擎；設定程式存檔即可。\n'
printf -- '- Windows 實機安裝、升級、反安裝與 UWP／Office 相容性仍需人工驗證。\n'
