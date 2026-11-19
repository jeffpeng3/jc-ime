#!/usr/bin/env python3
"""用 AJIMEE-Bench 200 題量測 Jinen v2（GGUF）的轉換準確率與延遲。

prompt 固定照模型卡：

    U+EE02<left_context>U+EE00<katakana_reading>U+EE01

沒有左文脈時整段 `U+EE02<left_context>` 省略（模型卡範例就是這樣寫）。
整段 prompt 先做 NFKC，再用 greedy（temperature 0、top_k 1）解碼。
讀音在 `ajimee_200.json` 是平假名（0c 介面用），模型卡要求片假名，
所以送進去前做平假名→片假名的一對一轉換。

用法：

    uv run --with llama-cpp-python python scripts/eval_jinen_ajimee.py --model xsmall
    uv run --with llama-cpp-python python scripts/eval_jinen_ajimee.py --model small

一次執行會跑兩組：

1. `nocontext`：全 200 題，左文脈一律空。這是與我們引擎、Google、
   AzooKey 對照的主數字。
2. `context`：只跑原始題目有左文脈的 100 題，餵入原始 `context_text`
   的尾端 64 字元，當作上限參考。

逐題結果寫到 `target/jinen-eval/`（已被 .gitignore 排除）。
題庫是 CC-BY-SA-3.0，只做評測，不進訓練。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import sys
import time
import unicodedata
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

DATA_ROOT = Path("/mnt/work/jc/jc-ime-data")
CORPUS = DATA_ROOT / "0c-testdata/ja/ajimee_200.json"
MODEL_DIR = DATA_ROOT / "models/external"

# AJIMEE-Bench JWTD_v2/v1/evaluation_items.json，dataset 的 source_sha256 就是這個檔案。
CONTEXT_URL = (
    "https://raw.githubusercontent.com/azooKey/AJIMEE-Bench/"
    "401666cd56d1a570c2021798b64b6da4396bfd45/JWTD_v2/v1/evaluation_items.json"
)
CONTEXT_SHA256 = "e9eb668fd6aa14b1e26436f429b5550108af0a1dfd443b8cea0bcb3ab3028fca"
CONTEXT_CACHE = Path("/tmp/opencode/jinen-eval/ajimee_evaluation_items.json")

MODELS = {
    "xsmall": "jinen-v2-xsmall/jinen-v2-xsmall-Q5_K_M.gguf",
    "small": "jinen-v2-small/jinen-v2-small-Q5_K_M.gguf",
}

INPUT_START = "\uee00"
OUTPUT_START = "\uee01"
CONTEXT_START = "\uee02"

CONTEXT_CHARS = 64
N_CTX = 1024
N_THREADS = 4
N_BATCH = 512
MAX_TOKENS = 256


def hiragana_to_katakana(text: str) -> str:
    """把平假名轉成片假名，其他字元原樣保留。"""
    out = []
    for ch in text:
        code = ord(ch)
        if 0x3041 <= code <= 0x3096:
            out.append(chr(code + 0x60))
        else:
            out.append(ch)
    return "".join(out)


def nfkc(text: str) -> str:
    return unicodedata.normalize("NFKC", text)


def build_prompt(reading: str, context: str) -> str:
    """照模型卡組 prompt，回傳已 NFKC 的字串。"""
    prompt = ""
    if context:
        prompt += CONTEXT_START + context[-CONTEXT_CHARS:]
    prompt += INPUT_START + hiragana_to_katakana(reading) + OUTPUT_START
    return nfkc(prompt)


def percentile(values: list[float], p: float) -> float:
    """最近秩百分位：排序後取第 ceil(p/100*n) 個。"""
    if not values:
        return 0.0
    ordered = sorted(values)
    rank = max(1, math.ceil(p / 100.0 * len(ordered)))
    return ordered[rank - 1]


def rss_kb(field_name: str) -> int:
    status = Path("/proc/self/status").read_text(encoding="utf-8")
    for line in status.splitlines():
        if line.startswith(field_name + ":"):
            return int(line.split()[1])
    return 0


def parse_affinity(text: str) -> set[int]:
    cpus: set[int] = set()
    for part in text.split(","):
        part = part.strip()
        if not part:
            continue
        if "-" in part:
            lo, hi = part.split("-", 1)
            cpus.update(range(int(lo), int(hi) + 1))
        else:
            cpus.add(int(part))
    return cpus


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def load_context_source(path: Path | None) -> dict[str, dict[str, Any]]:
    """取得原始 evaluation_items.json，依 index 建索引。"""
    cache = path or CONTEXT_CACHE
    if not cache.exists():
        cache.parent.mkdir(parents=True, exist_ok=True)
        print(f"下載 {CONTEXT_URL}", file=sys.stderr)
        with urllib.request.urlopen(CONTEXT_URL, timeout=60) as resp:
            cache.write_bytes(resp.read())
    got = sha256_file(cache)
    if got != CONTEXT_SHA256:
        raise SystemExit(f"{cache} 的 sha256 不符：期望 {CONTEXT_SHA256}，實際 {got}")
    items = json.loads(cache.read_text(encoding="utf-8"))
    return {str(item["index"]): item for item in items}


@dataclass
class Case:
    id: str
    subset: str  # "noctx" 或 "withctx"
    reading: str
    surface: str
    accept: list[str]
    context: str
    source_index: str
    extra: dict[str, Any] = field(default_factory=dict)


def load_cases(with_context: bool) -> list[Case]:
    corpus = json.loads(CORPUS.read_text(encoding="utf-8"))
    source = load_context_source(None)
    cases = []
    for item in corpus["items"]:
        src = source[item["source_index"]]
        raw_context = src.get("context_text") or ""
        has_context = bool(raw_context.strip())
        if with_context and not has_context:
            continue
        cases.append(
            Case(
                id=item["id"],
                subset="withctx" if has_context else "noctx",
                reading=item["reading"],
                surface=item["surface"],
                accept=item["accept"],
                context=raw_context if with_context else "",
                source_index=item["source_index"],
            )
        )
    return cases


def accepted_surfaces(case: Case) -> list[str]:
    """我們引擎的判定：surface 本身或 accept 任一項。"""
    values = [case.surface, *case.accept]
    return list(dict.fromkeys(values))


def summarize(rows: list[dict[str, Any]]) -> dict[str, Any]:
    n = len(rows)
    latencies = [row["latency_ms"] for row in rows]
    cpu_times = [row["cpu_ms"] for row in rows]
    return {
        "n": n,
        "raw_exact": sum(row["raw_exact"] for row in rows),
        "raw_exact_pct": round(100.0 * sum(row["raw_exact"] for row in rows) / n, 1) if n else 0.0,
        "nfkc_exact": sum(row["nfkc_exact"] for row in rows),
        "nfkc_exact_pct": round(100.0 * sum(row["nfkc_exact"] for row in rows) / n, 1) if n else 0.0,
        "accept_hit": sum(row["accept_hit"] for row in rows),
        "accept_hit_pct": round(100.0 * sum(row["accept_hit"] for row in rows) / n, 1) if n else 0.0,
        "accept_nfkc_hit": sum(row["accept_nfkc_hit"] for row in rows),
        "accept_nfkc_hit_pct": round(100.0 * sum(row["accept_nfkc_hit"] for row in rows) / n, 1) if n else 0.0,
        "truncated": sum(row["truncated"] for row in rows),
        "empty_output": sum(1 for row in rows if not row["output"]),
        "p50_ms": round(percentile(latencies, 50), 2),
        "p95_ms": round(percentile(latencies, 95), 2),
        "mean_ms": round(sum(latencies) / n, 2) if n else 0.0,
        "cpu_p50_ms": round(percentile(cpu_times, 50), 2),
        "cpu_p95_ms": round(percentile(cpu_times, 95), 2),
        "sum_completion_tokens": sum(row["completion_tokens"] for row in rows),
    }


def run(model_key: str, out_path: Path, affinity: set[int] | None) -> None:
    # 延遲量測前先匯入，import 時間不計入載入時間。
    import llama_cpp

    if affinity:
        current = os.sched_getaffinity(0)
        os.sched_setaffinity(0, affinity & current)
        print(f"CPU affinity: {sorted(os.sched_getaffinity(0))}", file=sys.stderr)

    started_at = time.strftime("%Y-%m-%dT%H:%M:%S%z")
    model_path = MODEL_DIR / MODELS[model_key]
    if not model_path.exists():
        raise SystemExit(f"找不到模型：{model_path}")

    rss_before_mb = rss_kb("VmRSS") / 1024.0
    loadavg_start = [round(v, 2) for v in os.getloadavg()]
    load_start = time.perf_counter()
    llm = llama_cpp.Llama(
        model_path=str(model_path),
        n_ctx=N_CTX,
        n_threads=N_THREADS,
        n_batch=N_BATCH,
        use_mmap=True,
        verbose=False,
    )
    load_seconds = time.perf_counter() - load_start
    rss_after_load_mb = rss_kb("VmRSS") / 1024.0

    # 暖機一次，不計入延遲；同時確認 greedy 解碼能正常結束。
    warm = build_prompt("テスト", "")
    llm.create_completion(
        warm,
        max_tokens=MAX_TOKENS,
        temperature=0.0,
        top_k=1,
        top_p=1.0,
        repeat_penalty=1.0,
        stop=None,
        echo=False,
    )

    def infer(case: Case) -> dict[str, Any]:
        prompt = build_prompt(case.reading, case.context)
        # 每題都清 KV cache，避免前一題的快取前綴影響 prefill 切批，
        # 讓逐題結果與題目順序無關。
        llm.reset()
        started = time.perf_counter()
        cpu_started = time.process_time()
        result = llm.create_completion(
            prompt,
            max_tokens=MAX_TOKENS,
            temperature=0.0,
            top_k=1,
            top_p=1.0,
            repeat_penalty=1.0,
            stop=None,
            echo=False,
        )
        latency_ms = (time.perf_counter() - started) * 1000.0
        cpu_ms = (time.process_time() - cpu_started) * 1000.0
        choice = result["choices"][0]
        output = choice["text"]
        completion_tokens = result["usage"]["completion_tokens"]
        finish = choice.get("finish_reason")
        accepted = accepted_surfaces(case)
        return {
            "id": case.id,
            "source_index": case.source_index,
            "subset": case.subset,
            "reading": case.reading,
            "surface": case.surface,
            "accept": case.accept,
            "context": case.context,
            "output": output,
            "raw_exact": output == case.surface,
            "nfkc_exact": nfkc(output) == nfkc(case.surface),
            "accept_hit": output in accepted,
            "accept_nfkc_hit": nfkc(output) in {nfkc(v) for v in accepted},
            "latency_ms": latency_ms,
            "cpu_ms": cpu_ms,
            "completion_tokens": completion_tokens,
            "truncated": finish == "length",
            "finish_reason": finish,
        }

    runs: dict[str, Any] = {}
    for run_name, with_context in (("nocontext", False), ("context", True)):
        cases = load_cases(with_context=with_context)
        rows = []
        for index, case in enumerate(cases, 1):
            rows.append(infer(case))
            if index % 50 == 0:
                print(f"[{model_key}/{run_name}] {index}/{len(cases)}", file=sys.stderr)
        runs[run_name] = {
            "description": (
                "全 200 題，左文脈空"
                if not with_context
                else f"原始有左文脈的 100 題，餵入尾端 {CONTEXT_CHARS} 字元"
            ),
            "all": summarize(rows),
            "noctx_subset": summarize([r for r in rows if r["subset"] == "noctx"]),
            "withctx_subset": summarize([r for r in rows if r["subset"] == "withctx"]),
            "items": rows,
        }
        print(f"[{model_key}/{run_name}] {json.dumps(runs[run_name]['all'], ensure_ascii=False)}", file=sys.stderr)

    payload = {
        "meta": {
            "model": model_key,
            "model_path": str(model_path),
            "model_sha256": sha256_file(model_path),
            "model_bytes": model_path.stat().st_size,
            "llama_cpp_python": llama_cpp.__version__,
            "llama_cpp_backend": llama_cpp.llama_cpp.llama_print_system_info().decode(
                "utf-8", "replace"
            ),
            "n_ctx": N_CTX,
            "n_threads": N_THREADS,
            "n_batch": N_BATCH,
            "max_tokens": MAX_TOKENS,
            "decoding": "greedy: temperature=0, top_k=1, top_p=1",
            "kv_cache": "reset before every item",
            "cpu_affinity": sorted(os.sched_getaffinity(0)),
            "prompt_format": "U+EE02<left_context>U+EE00<katakana_reading>U+EE01, NFKC, context tail 64 chars",
            "input_form": "ajimee reading (hiragana) -> katakana",
            "corpus": str(CORPUS),
            "corpus_sha256": sha256_file(CORPUS),
            "context_source": CONTEXT_URL,
            "context_source_sha256": CONTEXT_SHA256,
            "load_seconds": round(load_seconds, 3),
            "rss_before_load_mb": round(rss_before_mb, 1),
            "rss_after_load_mb": round(rss_after_load_mb, 1),
            "peak_rss_mb": round(rss_kb("VmHWM") / 1024.0, 1),
            "platform": {
                "machine": platform.machine(),
                "python": platform.python_version(),
                "nproc": os.cpu_count(),
            },
            "loadavg_at_start": loadavg_start,
            "loadavg_at_end": [round(v, 2) for v in os.getloadavg()],
            "started_at": started_at,
            "finished_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        },
        "runs": runs,
    }
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(payload, ensure_ascii=False, indent=1), encoding="utf-8")
    print(f"寫入 {out_path}", file=sys.stderr)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", choices=sorted(MODELS), required=True)
    parser.add_argument(
        "--out",
        type=Path,
        default=None,
        help="輸出 JSON 路徑，預設 target/jinen-eval/jinen-ajimee-<model>.json",
    )
    parser.add_argument(
        "--affinity",
        default="",
        help="把 4 個推論執行緒綁到的 CPU 清單（逗號或範圍），預設不綁；本機可用 5-8 綁到 X925",
    )
    args = parser.parse_args()
    out = args.out or Path("target/jinen-eval") / f"jinen-ajimee-{args.model}.json"
    affinity = parse_affinity(args.affinity) if args.affinity else None
    run(args.model, out, affinity)


if __name__ == "__main__":
    main()
