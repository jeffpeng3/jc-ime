#!/usr/bin/env python3
"""量測 ONNX 模型的檔案大小、延遲、RSS 與 fp32 數值一致性。

用法：
  python measure_onnx.py all                 # 逐一用子行程量測，輸出 JSON
  python measure_onnx.py worker <model> <precision>

延遲固定 intra_op_num_threads=8，每個組合跑 100 次（另加 10 次暖機）。
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

import numpy as np

QUANT = Path("/mnt/work/jc/jc-ime-data/models/quant")
INPUTS = Path("/tmp/opencode/quant-inputs")
REPORT = Path("/mnt/work/jc/wt/quant-study/docs/reports/quant-study-2026-11-20.json")

MODELS = {
    "ja-rerank": {"kind": "rerank", "input": "rerank.json"},
    "ja-punct": {"kind": "punct", "input": "punct.json"},
    "zh-select": {"kind": "select", "input": "zh-select.json"},
}
PRECISIONS = ("fp32", "int8", "int4")
ITERS = 100
WARMUP = 10
THREADS = 8


def load_inputs(model: str) -> list:
    return json.loads((INPUTS / MODELS[model]["input"]).read_text())


def rss_kb() -> int:
    for line in Path("/proc/self/status").read_text().splitlines():
        if line.startswith("VmRSS:"):
            return int(line.split()[1])
    raise RuntimeError("沒有 VmRSS")


def make_session(path: Path):
    import onnxruntime as ort

    so = ort.SessionOptions()
    so.intra_op_num_threads = THREADS
    so.inter_op_num_threads = 1
    so.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL
    return ort.InferenceSession(str(path), so, providers=["CPUExecutionProvider"])


def run_case(session, model: str, case) -> list[np.ndarray]:
    if model == "ja-rerank":
        seq = case["seq"]
        ids = np.asarray(case["ids"], dtype=np.int64).reshape(-1, seq)
        mask = np.asarray(case["mask"], dtype=np.int64).reshape(-1, seq)
        return session.run(None, {"input_ids": ids, "attention_mask": mask})
    if model == "ja-punct":
        ids = np.asarray([case["ids"]], dtype=np.int64)
        mask = np.ones_like(ids)
        return session.run(None, {"input_ids": ids, "attention_mask": mask})
    length = len(case["ids"])
    return session.run(
        None,
        {
            "ids": np.asarray([case["ids"]], dtype=np.int64),
            "bases": np.asarray([case["bases"]], dtype=np.int64),
            "tones": np.asarray([case["tones"]], dtype=np.int64),
            "types": np.asarray([case["types"]], dtype=np.int64),
            "positions": np.asarray([case["positions"]], dtype=np.int64),
            "mask": np.asarray(case["mask"], dtype=bool).reshape(1, 1, length, length),
            "proj_pos": np.asarray(case["proj_pos"], dtype=np.int64),
            "target_node": np.asarray(case["target_node"], dtype=np.int64),
            "target_ids": np.asarray(case["target_ids"], dtype=np.int64),
        },
    )


def per_case_scores(model: str, case, out: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    """把原始輸出整理成「每個候選一個分數」。"""
    if model == "ja-rerank":
        return out, np.arange(len(out))
    if model == "ja-punct":
        # 以每個 token 的最大 logit 當排序依據，另附 argmax 供一致率使用。
        if out.ndim == 3:
            out = out[0]
        return out.max(axis=1), out.argmax(axis=1)
    raw = out
    count = len(case["candidates"])
    scores = np.full(count, -np.inf)
    for row, value in enumerate(raw):
        idx = row % count
        scores[idx] = max(scores[idx], value)
    return scores, np.arange(count)


def numeric_compare(model: str, ref: dict, target: dict) -> dict:
    from scipy.stats import spearmanr

    assert [c.get("id") for c in ref["cases"]] == [c.get("id") for c in target["cases"]]
    max_abs = 0.0
    flat_ref: list[float] = []
    flat_tgt: list[float] = []
    spearmans: list[float] = []
    top1 = 0
    exact = 0
    argmax_hits = 0
    argmax_total = 0
    for a, b in zip(ref["cases"], target["cases"]):
        ra = np.asarray(a["raw"], dtype=np.float64)
        rb = np.asarray(b["raw"], dtype=np.float64)
        max_abs = max(max_abs, float(np.max(np.abs(ra - rb))))
        flat_ref.extend(ra.ravel().tolist())
        flat_tgt.extend(rb.ravel().tolist())
        sa, _ = per_case_scores(model, a["case"], ra)
        sb, _ = per_case_scores(model, a["case"], rb)
        if len(sa) > 1 and np.std(sa) > 0 and np.std(sb) > 0:
            rho = spearmanr(sa, sb).statistic
            if np.isfinite(rho):
                spearmans.append(float(rho))
        if int(np.argmax(sa)) == int(np.argmax(sb)):
            top1 += 1
        if np.array_equal(np.argsort(-sa), np.argsort(-sb)):
            exact += 1
        if model == "ja-punct":
            # 逐 token 的 argmax（每 4 個一列）一致率。
            la = ra.reshape(-1, 4)
            lb = rb.reshape(-1, 4)
            argmax_hits += int((la.argmax(axis=1) == lb.argmax(axis=1)).sum())
            argmax_total += la.shape[0]
    rho_all = spearmanr(flat_ref, flat_tgt).statistic if len(flat_ref) > 1 else float("nan")
    return {
        "cases": len(ref["cases"]),
        "max_abs_error": max_abs,
        "spearman_mean_per_case": float(np.mean(spearmans)) if spearmans else float("nan"),
        "spearman_all_values": float(rho_all),
        "top1_agreement": top1 / len(ref["cases"]),
        "exact_order_agreement": exact / len(ref["cases"]),
        "argmax_agreement": (argmax_hits / argmax_total) if argmax_total else None,
    }


def worker(model: str, precision: str) -> dict:
    kind = MODELS[model]["kind"]
    path = QUANT / model / precision / "model.onnx"
    cases = load_inputs(model)
    result = {
        "model": model,
        "precision": precision,
        "file_bytes": path.stat().st_size,
        "file_mb": path.stat().st_size / 1e6,
        "kind": kind,
    }
    before = rss_kb()
    session = make_session(path)
    after_load = rss_kb()
    result["rss_load_mb"] = (after_load - before) / 1024

    # 延遲：固定一筆代表輸入。
    if model == "ja-rerank":
        bench_case = cases[8]
    elif model == "ja-punct":
        bench_case = next(c for c in cases if len(c["ids"]) == 256)
    else:
        bench_case = max(cases, key=lambda c: len(c["ids"]))
    for _ in range(WARMUP):
        run_case(session, model, bench_case)
    from time import perf_counter

    rounds = []
    for _ in range(3):
        times = []
        for _ in range(ITERS):
            start = perf_counter()
            run_case(session, model, bench_case)
            times.append((perf_counter() - start) * 1000)
        arr = np.asarray(times)
        rounds.append(
            {
                "p50": float(np.percentile(arr, 50)),
                "p95": float(np.percentile(arr, 95)),
                "mean": float(arr.mean()),
            }
        )
    best = min(rounds, key=lambda r: r["p50"])
    result["latency_ms"] = {
        "p50": best["p50"],
        "p95": best["p95"],
        "mean": best["mean"],
        "p50_median": float(np.median([r["p50"] for r in rounds])),
        "p95_median": float(np.median([r["p95"] for r in rounds])),
        "selection": "三次中 p50 最低的一輪",
        "rounds": rounds,
        "iters": ITERS,
    }
    result["bench_shape"] = (
        f"rows={bench_case['rows']},seq={bench_case['seq']}"
        if model == "ja-rerank"
        else (
            f"seq={len(bench_case['ids'])}"
            if model == "ja-punct"
            else f"L={len(bench_case['ids'])},rows={len(bench_case['target_node'])}"
        )
    )

    # 數值：fp32 與量化一起載入太吃記憶體，只存 raw 輸出，由父行程比較。
    raw_cases = []
    for case in cases:
        out = run_case(session, model, case)
        raw = np.asarray(out[0])
        if raw.ndim == 3:
            raw = raw[0]
        raw_cases.append({"id": case.get("id") or case.get("source"), "case": case, "raw": raw.tolist()})
    result["cases"] = raw_cases
    return result


def orchestrate() -> None:
    data = {"threads": THREADS, "iters": ITERS, "warmup": WARMUP, "models": {}}
    data["loadavg"] = [round(v, 2) for v in os.getloadavg()]
    for model in MODELS:
        data["models"][model] = {}
        for precision in PRECISIONS:
            path = QUANT / model / precision / "model.onnx"
            if not path.is_file():
                data["models"][model][precision] = {"missing": str(path)}
                continue
            print(f"[measure] {model} {precision}", flush=True)
            proc = subprocess.run(
                [sys.executable, __file__, "worker", model, precision],
                capture_output=True,
                text=True,
            )
            if proc.returncode != 0:
                data["models"][model][precision] = {
                    "error": proc.stderr.strip().splitlines()[-1] if proc.stderr.strip() else "unknown"
                }
                continue
            result = json.loads(proc.stdout)
            data["models"][model][precision] = {
                k: v for k, v in result.items() if k != "cases"
            }
            data["models"][model][precision]["raw_file"] = f"raw/{model}-{precision}.json"
            raw_path = REPORT.parent / "raw" / f"{model}-{precision}.json"
            raw_path.parent.mkdir(parents=True, exist_ok=True)
            raw_path.write_text(json.dumps(result["cases"]))

    for model in MODELS:
        ref_path = REPORT.parent / "raw" / f"{model}-fp32.json"
        if not ref_path.is_file():
            continue
        ref = {"cases": json.loads(ref_path.read_text())}
        for precision in PRECISIONS:
            target_path = REPORT.parent / "raw" / f"{model}-{precision}.json"
            if not target_path.is_file():
                continue
            target = {"cases": json.loads(target_path.read_text())}
            metric = numeric_compare(model, ref, target)
            data["models"][model][precision]["numeric_vs_fp32"] = metric
    REPORT.write_text(json.dumps(data, ensure_ascii=False, indent=2))
    print(f"寫出 {REPORT}")


def main() -> int:
    if len(sys.argv) >= 2 and sys.argv[1] == "worker":
        model, precision = sys.argv[2], sys.argv[3]
        json.dump(worker(model, precision), sys.stdout)
        return 0
    orchestrate()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
