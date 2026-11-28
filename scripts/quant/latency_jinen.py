#!/usr/bin/env python3
"""GGUF 延遲量測：每個量化跑 3 輪、每輪 100 題，取 p50/p95 的中位數。

每輪都在獨立子行程載入模型，避免前一個量化的記憶體與快取影響。
延遲只看 nocontext prompt，固定 n_threads=8、greedy、n_ctx=1024。
"""

from __future__ import annotations

import json
import statistics
import subprocess
import sys
import time
import unicodedata
from pathlib import Path

ROOT = Path("/mnt/work/jc/jc-ime-data/models/quant/jinen-v2-xsmall")
CORPUS = Path("/mnt/work/jc/jc-ime-data/0c-testdata/ja/ajimee_200.json")
OUT = Path("/tmp/opencode/jinen-latency.json")
QUANTS = ("f16", "q8_0", "q5_k_m", "q4_k_m")
ITEMS = 100


def ja_katakana(text: str) -> str:
    return "".join(chr(ord(c) + 0x60) if 0x3041 <= ord(c) <= 0x3096 else c for c in text)


def prompt_of(reading: str) -> str:
    return unicodedata.normalize("NFKC", "\uee00" + ja_katakana(reading) + "\uee01")


def one_round(quant: str, readings: list[str]) -> dict:
    from llama_cpp import Llama

    llm = Llama(
        model_path=str(ROOT / quant / "model.gguf"),
        n_ctx=1024,
        n_threads=8,
        n_batch=512,
        verbose=False,
    )
    # 暖機
    llm.create_completion(prompt_of("テスト"), max_tokens=16, temperature=0.0, top_k=1)
    times = []
    for reading in readings:
        start = time.perf_counter()
        llm.create_completion(
            prompt_of(reading),
            max_tokens=max(8, len(reading) + 8),
            temperature=0.0,
            top_k=1,
            top_p=1.0,
            repeat_penalty=1.0,
        )
        times.append((time.perf_counter() - start) * 1000)
    llm.close()
    times.sort()
    n = len(times)
    return {
        "p50": times[int(0.50 * (n - 1))],
        "p95": times[int(0.95 * (n - 1))],
    }


def main() -> int:
    if len(sys.argv) >= 2:
        readings = [
            item["reading"]
            for item in json.loads(CORPUS.read_text())["items"][:ITEMS]
        ]
        json.dump(one_round(sys.argv[1], readings), sys.stdout)
        return 0
    result = {q: [] for q in QUANTS}
    for _ in range(3):
        for quant in QUANTS:
            proc = subprocess.run(
                [sys.executable, __file__, quant], capture_output=True, text=True
            )
            if proc.returncode != 0:
                raise SystemExit(proc.stderr[-400:])
            result[quant].append(json.loads(proc.stdout))
    summary = {}
    for quant, rounds in result.items():
        best = min(rounds, key=lambda r: r["p50"])
        summary[quant] = {
            "rounds": rounds,
            "p50": best["p50"],
            "p95": best["p95"],
            "p50_median": statistics.median(r["p50"] for r in rounds),
            "p95_median": statistics.median(r["p95"] for r in rounds),
            "selection": "三次中 p50 最低的一輪",
        }
    OUT.write_text(json.dumps(summary, ensure_ascii=False, indent=2))
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
