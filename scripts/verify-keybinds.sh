#!/usr/bin/env bash
# 手動驗證：快捷鍵由引擎解析，且設定檔熱重載後立刻生效。
#
#   1. 起 jc-engine（socket 模式）並送 ctrl+alt+,
#   2. 確認 handled=true 且 state 有符號選單候選
#   3. 改 settings.json 的 symbol_menu
#   4. 1.5 秒後送舊鍵（應放行）與新鍵（應命中），並確認 revision 變了
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

engine=${JC_ENGINE:-$root/target/debug/jc-engine}
if [[ ! -x $engine ]]; then
    echo "建置 jc-engine..."
    cargo build -p jc-engine
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/jc-keybinds.XXXXXX")
sock=$tmp/engine.sock
settings=$tmp/settings.json
log=$tmp/engine.log
pid=

cleanup() {
    status=$?
    if [[ -n ${pid} ]] && kill -0 "$pid" 2>/dev/null; then
        kill "$pid" 2>/dev/null || true
        wait "$pid" 2>/dev/null || true
    fi
    if [[ $status -ne 0 ]]; then
        echo "引擎 log 留在 $log" >&2
    else
        rm -rf "$tmp"
    fi
}
trap cleanup EXIT

cat >"$settings" <<'JSON'
{"keybinds":{"symbol_menu":"ctrl+alt+,","emoji_palette":"ctrl+alt+."}}
JSON

"$engine" --allow-empty-data --settings "$settings" --socket "$sock" \
    >"$log" 2>&1 &
pid=$!

uv run --no-project --quiet python - "$sock" "$settings" <<'PY'
import json
import os
import socket
import struct
import sys
import time

sock_path, settings_path = sys.argv[1], sys.argv[2]


def connect(deadline=5.0):
    # jc-windows 在 Linux 把 --socket 當 abstract namespace 名稱（不建檔案）。
    address = b"\0" + sock_path.encode()
    end = time.monotonic() + deadline
    while time.monotonic() < end:
        try:
            client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            client.connect(address)
            return client
        except OSError:
            time.sleep(0.02)
    raise SystemExit("5 秒內連不上引擎 socket")


def send(client, payload):
    body = json.dumps(payload).encode()
    client.sendall(struct.pack("<I", len(body)) + body)


def read_exact(client, size):
    chunks = []
    while size:
        chunk = client.recv(size)
        if not chunk:
            raise SystemExit("引擎提前關閉連線")
        chunks.append(chunk)
        size -= len(chunk)
    return b"".join(chunks)


def recv(client):
    (size,) = struct.unpack("<I", read_exact(client, 4))
    return json.loads(read_exact(client, size))


def call(client, payload):
    send(client, payload)
    return recv(client)


def key(client, text, ctrl=False, alt=False, shift=False):
    return call(
        client,
        {
            "op": "key",
            "key": {
                "code": {"kind": "char", "value": text},
                "text": text,
                "shift": shift,
                "ctrl": ctrl,
                "alt": alt,
            },
        },
    )


def check(condition, message):
    if not condition:
        raise SystemExit(f"FAIL: {message}")
    print(f"ok: {message}")


client = connect()
ready = call(client, {"op": "hello", "protocol": 3, "client": "verify-keybinds"})
check(ready.get("op") == "ready", "hello 回 ready")
check(ready.get("bindings", {}).get("symbol_menu") == "ctrl+alt+,", "預設 symbol_menu 綁定")

state = key(client, ",", ctrl=True, alt=True)
check(state.get("handled") is True, "ctrl+alt+, handled=true")
check(bool(state.get("candidates")), "state 有符號選單候選")

health = call(client, {"op": "health"})
before = health["info"].get("settings_revision", 0)
print(f"ok: 初始 settings_revision={before}")

new = {
    "keybinds": {
        "symbol_menu": "ctrl+alt+s",
        "emoji_palette": "ctrl+alt+.",
    }
}
tmp = settings_path + ".new"
with open(tmp, "w", encoding="utf-8") as handle:
    json.dump(new, handle)
os.replace(tmp, settings_path)
print("ok: 已改寫 settings.json（symbol_menu=ctrl+alt+s）")

time.sleep(1.5)
health = call(client, {"op": "health"})
deadline = time.monotonic() + 3.0
while health["info"].get("settings_revision", 0) == before and time.monotonic() < deadline:
    time.sleep(0.1)
    health = call(client, {"op": "health"})
after = health["info"].get("settings_revision", 0)
check(after > before, f"熱重載 revision {before} -> {after}")
check(health["info"]["bindings"]["symbol_menu"] == "ctrl+alt+s", "bindings 換成新 chord")
check(not health["info"].get("settings_error"), "熱重載沒有錯誤")

old = key(client, ",", ctrl=True, alt=True)
check(old.get("handled") is False, "舊鍵 ctrl+alt+, 已放行")

new_state = key(client, "s", ctrl=True, alt=True)
check(new_state.get("handled") is True, "新鍵 ctrl+alt+s handled=true")
check(bool(new_state.get("candidates")), "新鍵也真的開符號選單")

stable = after
bad = {
    "keybinds": {
        "symbol_menu": "a",
        "emoji_palette": "ctrl+alt+.",
    }
}
tmp = settings_path + ".new"
with open(tmp, "w", encoding="utf-8") as handle:
    json.dump(bad, handle)
os.replace(tmp, settings_path)
print("ok: 已改寫 settings.json（合法 JSON，但 symbol_menu=a 缺修飾鍵）")

time.sleep(1.5)
health = call(client, {"op": "health"})
deadline = time.monotonic() + 3.0
while not health["info"].get("settings_error") and time.monotonic() < deadline:
    time.sleep(0.1)
    health = call(client, {"op": "health"})
check(
    health["info"].get("settings_revision", 0) == stable,
    "壞 chord 不增加 revision",
)
check(
    health["info"]["bindings"]["symbol_menu"] == "ctrl+alt+s",
    "壞 chord 保留舊綁定",
)
check(bool(health["info"].get("settings_error")), "health 回報 settings_error")

still = key(client, "s", ctrl=True, alt=True)
check(still.get("handled") is True, "壞 chord 之後舊 chord 仍 handled=true")
check(bool(still.get("candidates")), "舊 chord 仍真的開符號選單")

call(client, {"op": "shutdown"})
client.close()
print("PASS: 快捷鍵、熱重載與壞 chord 保留舊設定驗證通過")
PY

wait "$pid"
pid=
