#!/usr/bin/env bash
# 清理 GitHub Actions 快取：同一 family 只留最新 N 份，或超過 T 天沒被讀取就刪。
#
# 兩條規則取 OR，任一成立就刪。family 的算法是把 key 尾端的十六進位雜湊拿掉，
# 因此 Linux／Windows、ci／release 這類共用前綴不會被併成同一個 family。
# 剛建立不久的快取一律不刪（預設 6 小時），避免刪到並行 run 正在用的項目。
#
# 用法：
#   GH_TOKEN=... GH_REPO=owner/repo scripts/cache-prune.sh [--dry-run] [--skip-recent-hours N]
#   CACHE_JSON=scripts/fixtures/cache-prune.json scripts/cache-prune.sh --dry-run
#
# 環境變數：
#   CACHE_JSON          改讀本機 JSON（gh cache list --json 的輸出），不呼叫 gh
#   KEEP_PER_FAMILY     每個 family 保留最新幾份，預設 2
#   MAX_IDLE_DAYS       幾天沒被讀取就刪，預設 7
#   SKIP_RECENT_HOURS   建立未滿幾小時一律不刪，預設 6
#   PRUNE_NOW           以指定時間當「現在」（ISO 8601），測試用，預設系統時間
#   GITHUB_STEP_SUMMARY 有設就追加 Markdown 總結
set -euo pipefail

KEEP_PER_FAMILY=${KEEP_PER_FAMILY:-2}
MAX_IDLE_DAYS=${MAX_IDLE_DAYS:-7}
SKIP_RECENT_HOURS=${SKIP_RECENT_HOURS:-6}
DRY_RUN=0

usage() {
  # 印出檔頭註解：第 2 行到 `set -euo` 之前。
  sed -n '2,/^set -euo/{/^set -euo/d;p}' "$0"
}

while (($# > 0)); do
  case $1 in
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --skip-recent-hours)
      SKIP_RECENT_HOURS=${2:?--skip-recent-hours 需要一個數字}
      shift 2
      ;;
    --skip-recent-hours=*)
      SKIP_RECENT_HOURS=${1#*=}
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "未知參數：$1（可用：--dry-run、--skip-recent-hours N）" >&2
      exit 2
      ;;
  esac
done

for v in KEEP_PER_FAMILY MAX_IDLE_DAYS SKIP_RECENT_HOURS; do
  if ! [[ ${!v} =~ ^[0-9]+$ ]]; then
    echo "$v 必須是非負整數，目前是 ${!v}" >&2
    exit 2
  fi
done

if ! command -v jq >/dev/null; then
  echo "需要 jq" >&2
  exit 1
fi

if [[ -n ${CACHE_JSON:-} ]]; then
  if [[ ! -r $CACHE_JSON ]]; then
    echo "讀不到 CACHE_JSON：$CACHE_JSON" >&2
    exit 1
  fi
  raw=$(cat "$CACHE_JSON")
else
  if [[ -z ${GH_TOKEN:-} || -z ${GH_REPO:-} ]]; then
    echo "需要 GH_TOKEN 與 GH_REPO，或改用 CACHE_JSON 讀本機資料" >&2
    exit 1
  fi
  raw=$(gh cache list --repo "$GH_REPO" --limit 100 \
    --json id,key,sizeInBytes,createdAt,lastAccessedAt)
fi

if ! jq -e 'type == "array"' >/dev/null 2>&1 <<<"$raw"; then
  echo "快取清單不是 JSON 陣列" >&2
  exit 1
fi

NOW_ISO=${PRUNE_NOW:-$(date -u +%Y-%m-%dT%H:%M:%SZ)}
if ! now_epoch=$(date -u -d "$NOW_ISO" +%s 2>/dev/null); then
  echo "PRUNE_NOW 不是可解析的時間：$NOW_ISO" >&2
  exit 2
fi

# jq 只做判定，不負責排版。family 規則：
#   1. 尾端 >= 40 位十六進位，前面可有 - 或 _，直接視為雜湊。
#   2. 尾端 16～39 位十六進位且同時含數字與字母，也視為雜湊（rust-cache 用 16 位）。
#      要求同時含數字與字母，避免把 `-2026010112000000` 這種純數字時間戳切掉。
read -r -d '' JQ_PROGRAM <<'JQ' || true
def family:
  . as $key
  | if ($key | test("[-_][0-9a-fA-F]{40,}$")) then
      $key | sub("[-_][0-9a-fA-F]{40,}$"; "")
    elif ($key | test("[-_][0-9a-fA-F]{16,39}$")) then
      ($key | capture("(?<base>.*)[-_](?<hash>[0-9a-fA-F]{16,39})$")) as $m
      | if (($m.hash | test("[0-9]")) and ($m.hash | test("[a-fA-F]"))) then
          $m.base
        else
          $key
        end
    else
      $key
    end;

def epoch:
  sub("\\.[0-9]+"; "") | fromdateiso8601;

def min_utc:
  .[0:16] | sub("T"; " ");

($now) as $now
| [ .[]
    | . + {
        family: (.key | family),
        created_ts: (.createdAt | epoch),
        size: (.sizeInBytes // 0),
      }
    | . + {accessed_ts: ((.lastAccessedAt // .createdAt) | epoch)}
  ]
| group_by(.family)
| map(sort_by([-.created_ts, .id]) | to_entries | map(.value + {rank: (.key + 1)}))
| add // []
| map(
    . as $c
    | ((($now - $c.accessed_ts) / 86400 * 10 | floor) / 10) as $idle
    | ((($now - $c.created_ts) / 3600 * 10 | floor) / 10) as $age
    | ($age < $skip_recent) as $too_new
    | ([
        (if $c.rank > $keep then "family 第 \($c.rank) 份 > \($keep)" else empty end),
        (if $idle > $maxidle then "閒置 \($idle) 天 > \($maxidle)" else empty end)
      ] | join("；")) as $why
    | $c
    + {
        idle_days: $idle,
        age_hours: $age,
        too_new: $too_new,
        last_used: (if $c.lastAccessedAt then ($c.lastAccessedAt | min_utc)
                    else "從未讀取（建立於 \($c.createdAt | min_utc)）" end),
        reason: (
          if $too_new and ($c.rank > $keep or $idle > $maxidle) then
            "建立未滿 \($skip_recent) 小時（\($age) 小時），暫不刪；\($why)"
          elif $why == "" then "最新且近期有讀取"
          else $why end
        ),
        delete: (($c.rank > $keep or $idle > $maxidle) and ($too_new | not)),
      }
  )
| {
    delete: ([.[] | select(.delete)] | sort_by([-.created_ts, .id])),
    keep: ([.[] | select(.delete | not)] | sort_by([-.created_ts, .id])),
    skipped: ([.[] | select(.too_new and (.rank > $keep or .idle_days > $maxidle))]
              | sort_by([-.created_ts, .id])),
  }
JQ

plan=$(jq -c \
  --argjson now "$now_epoch" \
  --argjson keep "$KEEP_PER_FAMILY" \
  --argjson maxidle "$MAX_IDLE_DAYS" \
  --argjson skip_recent "$SKIP_RECENT_HOURS" \
  "$JQ_PROGRAM" <<<"$raw")

total=$(jq '.delete + .keep | length' <<<"$plan")
total_bytes=$(jq '.delete + .keep | map(.size) | add // 0' <<<"$plan")
planned=$(jq '.delete | length' <<<"$plan")
planned_bytes=$(jq '.delete | map(.size) | add // 0' <<<"$plan")
kept=$(jq '.keep | length' <<<"$plan")
kept_bytes=$(jq '.keep | map(.size) | add // 0' <<<"$plan")
skipped=$(jq '.skipped | length' <<<"$plan")

mb() { awk -v b="$1" 'BEGIN { printf "%.1f", b / 1048576 }'; }

mode_text() {
  if ((DRY_RUN)); then echo "dry-run（未刪除任何快取）"; else echo "實際刪除"; fi
}

print_table() {
  local label=$1 rows=$2
  local n
  n=$(jq 'length' <<<"$rows")
  echo
  echo "== $label：$n 筆 =="
  if ((n == 0)); then
    echo "（無）"
    return
  fi
  printf '%s | %s | %s | %s\n' "key" "大小(MB)" "最後使用(UTC)" "判定原因"
  jq -r '.[] | [.key, (.size / 1048576 * 10 | round / 10 | tostring), .last_used, .reason] | @tsv' <<<"$rows" |
    while IFS=$'\t' read -r key size used reason; do
      printf '%s | %s | %s | %s\n' "$key" "$size" "$used" "$reason"
    done
}

DELETE_ROWS=$(jq -c '.delete' <<<"$plan")
KEEP_ROWS=$(jq -c '.keep' <<<"$plan")

echo "快取清理：family 上限 $KEEP_PER_FAMILY 份，閒置上限 $MAX_IDLE_DAYS 天，建立未滿 $SKIP_RECENT_HOURS 小時不動（現在 $NOW_ISO）"
print_table "要刪除" "$DELETE_ROWS"
print_table "保留" "$KEEP_ROWS"

deleted=0
freed=0
failed=0
if ((planned > 0 && !DRY_RUN)); then
  echo
  echo "== 刪除 =="
  while IFS=$'\t' read -r id size key; do
    if gh cache delete "$id" --repo "$GH_REPO"; then
      deleted=$((deleted + 1))
      freed=$((freed + size))
      echo "已刪除 $key（$id）"
    else
      failed=$((failed + 1))
      echo "刪除失敗 $key（$id）" >&2
    fi
  done < <(jq -r '.delete[] | [.id, .size, .key] | @tsv' <<<"$plan")
else
  # dry-run 或無事可做時，大小口徑仍用計畫值，和實際刪除一致。
  freed=$planned_bytes
  deleted=$planned
fi

echo
echo "== 總結 =="
echo "模式：$(mode_text)"
echo "掃描：$total 筆，$(mb "$total_bytes") MB"
if ((DRY_RUN)); then
  echo "待刪除：$planned 筆，$(mb "$planned_bytes") MB"
else
  echo "刪除成功：$deleted 筆，$(mb "$freed") MB"
  echo "刪除失敗：$failed 筆"
fi
echo "保留：$kept 筆，$(mb "$kept_bytes") MB"
echo "因建立未滿 $SKIP_RECENT_HOURS 小時而跳過：$skipped 筆"

if [[ -n ${GITHUB_STEP_SUMMARY:-} ]]; then
  {
    echo "### 快取清理：$(mode_text)"
    echo
    echo "規則：同 family 留最新 $KEEP_PER_FAMILY 份，或超過 $MAX_IDLE_DAYS 天沒被讀取即刪（OR）；"
    echo "建立未滿 $SKIP_RECENT_HOURS 小時一律不刪。"
    echo
    echo "判定時間：$NOW_ISO"
    echo
    echo "| 項目 | 筆數 | 大小 |"
    echo "|---|---:|---:|"
    echo "| 掃描 | $total | $(mb "$total_bytes") MB |"
    if ((DRY_RUN)); then
      echo "| 待刪除 | $planned | $(mb "$planned_bytes") MB |"
    else
      echo "| 刪除成功 | $deleted | $(mb "$freed") MB |"
      echo "| 刪除失敗 | $failed | - |"
    fi
    echo "| 保留 | $kept | $(mb "$kept_bytes") MB |"
    echo "| 因太新跳過 | $skipped | - |"
    echo
    echo "| key | 大小(MB) | 最後使用(UTC) | 判定原因 |"
    echo "|---|---:|---|---|"
    if ((planned == 0)); then
      echo "| （無） | | | |"
    else
      jq -r '.[] | "| `\(.key)` | \(.size / 1048576 * 10 | round / 10) | \(.last_used) | \(.reason) |"' <<<"$DELETE_ROWS"
    fi
  } >>"$GITHUB_STEP_SUMMARY"
fi

if ((failed > 0)); then
  exit 1
fi
