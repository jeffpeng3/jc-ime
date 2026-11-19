#!/usr/bin/env python3
"""端到端：日文標點量化後與 fp32 的輸出一致率。

用 jc-cli convert 逐句跑同一批 modern-500 讀音，比較插入標點後的輸出。
"""

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path

ROOT = Path("/mnt/work/jc/wt/quant-study")
TESTDATA = Path("/mnt/work/jc/jc-ime-data/0c-testdata")
OVERLAY = Path("/mnt/work/jc/jc-ime-data/models/quant/overlay")
OUT = Path("/tmp/opencode/punct-e2e.json")


def run_precision(precision: str, readings: list[str]) -> list[str]:
    env = dict(os.environ)
    env["ORT_DYLIB_PATH"] = "/tmp/opencode/libonnxruntime.so.1.32.0"
    env["JC_MODELS"] = str(OVERLAY / precision)
    out = []
    for text in readings:
        proc = subprocess.run(
            [
                str(ROOT / "target/release/jc-cli"),
                "convert",
                text,
                "--punct",
                "on",
                "--select-model",
                "off",
                "--n",
                "1",
                "--json",
            ],
            capture_output=True,
            text=True,
            env=env,
        )
        if proc.returncode != 0:
            raise RuntimeError(proc.stderr[-400:])
        out.append(json.loads(proc.stdout)[0]["surface"])
    return out


def char_agreement(a: str, b: str) -> float:
    same = sum(1 for x, y in zip(a, b) if x == y)
    total = max(len(a), len(b))
    return same / total if total else 1.0


def main() -> None:
    corpus = json.loads((TESTDATA / "ja/modern_500.json").read_text())["items"][:100]
    readings = [item["reading"] for item in corpus]
    outputs = {p: run_precision(p, readings) for p in ("fp32", "int8", "int4")}
    result = {"n": len(readings), "precisions": {}}
    for p in ("fp32", "int8", "int4"):
        result["precisions"][p] = outputs[p]
    for p in ("int8", "int4"):
        exact = sum(1 for a, b in zip(outputs["fp32"], outputs[p]) if a == b)
        chars = [char_agreement(a, b) for a, b in zip(outputs["fp32"], outputs[p])]
        result[f"{p}_vs_fp32"] = {
            "sentence_exact_rate": exact / len(readings),
            "char_agreement_mean": sum(chars) / len(chars),
            "changed_sentences": len(readings) - exact,
        }
    OUT.write_text(json.dumps(result, ensure_ascii=False, indent=2))
    print(json.dumps({k: v for k, v in result.items() if k.endswith("vs_fp32")}, indent=2))


if __name__ == "__main__":
    main()
