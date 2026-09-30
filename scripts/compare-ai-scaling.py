#!/usr/bin/env python3
"""
Hologram-AI Operations and Capabilities Scaling Benchmark
PrismPM Declarative Architecture vs Non-PrismPM Imperative Architecture

Measures and contrasts:
1. Working Set Containment (WS-1..WS-3) across Model Sizes (1B, 3B, 8B, 70B) and Context Windows (4k, 32k, 128k)
2. Prefix KV-Cache Token Elision Memory Savings (0%, 50%, 80%)
3. 4-Way Fused Operator DRAM Bandwidth Reductions (FU-1..FU-4)
4. OS Swap Thrashing Failure Points under Imperative Execution
5. Inductive Command Dispatch Latency & Throughput
"""

import json
import os
import subprocess
import sys
import time
from pathlib import Path

BINARY_PATH = Path("target/release/hologram")
if not BINARY_PATH.exists():
    BINARY_PATH = Path("target/debug/hologram")


def run_ai_compare(model: str, context_len: int, prefix_tokens: int, budget_gb: int) -> dict:
    cmd = [
        str(BINARY_PATH),
        "--prism",
        "ai",
        "compare",
        "--model", model,
        "--context-length", str(context_len),
        "--prefix-tokens", str(prefix_tokens),
        "--memory-budget-gb", str(budget_gb),
        "--json",
    ]
    t0 = time.perf_counter()
    proc = subprocess.run(cmd, capture_output=True, text=True)
    t1 = time.perf_counter()
    if proc.returncode != 0:
        print(f"Error running ai compare: {proc.stderr}", file=sys.stderr)
        return {}
    res = json.loads(proc.stdout)
    res["execution_latency_ms"] = round((t1 - t0) * 1000.0, 3)
    return res


def main():
    print("=" * 80)
    print("  Hologram-AI Operations & Capabilities Benchmark: PrismPM vs Non-PrismPM")
    print(f"  Binary: {BINARY_PATH} ({BINARY_PATH.stat().st_size / 1_000_000:.1f} MB)")
    print("=" * 80)
    print()

    # Scaling Matrix
    # (Model, Context Length, Prefix Ratio, Memory Budget GB)
    scenarios = [
        # Edge Workloads (8 GB Budget)
        ("1b", 4096, 0.5, 8, "Llama-3.2-1B @ 4k Context (Edge IoT)"),
        ("1b", 32768, 0.5, 8, "Llama-3.2-1B @ 32k Context (Edge IoT)"),
        ("3b", 4096, 0.5, 8, "Llama-3.2-3B @ 4k Context (Mobile/Edge)"),
        ("3b", 32768, 0.8, 8, "Llama-3.2-3B @ 32k Context (Mobile/Edge)"),

        # Workstation Workloads (16 GB / 32 GB Budget)
        ("7b", 4096, 0.5, 16, "Llama-2-7B @ 4k Context (Standard Workstation)"),
        ("7b", 32768, 0.5, 16, "Llama-2-7B @ 32k Context (Standard Workstation)"),
        ("7b", 131072, 0.0, 16, "Llama-2-7B @ 128k Full Context (0% Prefix Elision)"),
        ("7b", 131072, 0.8, 16, "Llama-2-7B @ 128k Full Context (80% Prefix Elision)"),
        ("8b", 4096, 0.5, 16, "Llama-3.1-8B @ 4k Context (Standard Workstation)"),
        ("8b", 32768, 0.5, 16, "Llama-3.1-8B @ 32k Context (Extended Context)"),
        ("8b", 131072, 0.0, 16, "Llama-3.1-8B @ 128k Full Context (0% Prefix Elision)"),
        ("8b", 131072, 0.5, 16, "Llama-3.1-8B @ 128k Full Context (50% Prefix Elision)"),
        ("8b", 131072, 0.8, 16, "Llama-3.1-8B @ 128k Full Context (80% Prefix Elision)"),
        ("13b", 4096, 0.5, 32, "Llama-2-13B @ 4k Context (High-End Workstation)"),
        ("13b", 32768, 0.5, 32, "Llama-2-13B @ 32k Context (High-End Workstation)"),
        ("13b", 131072, 0.0, 32, "Llama-2-13B @ 128k Full Context (0% Prefix Elision)"),
        ("13b", 131072, 0.8, 32, "Llama-2-13B @ 128k Full Context (80% Prefix Elision)"),

        # High-Memory / Server Workloads (64 GB Budget)
        ("70b", 4096, 0.5, 64, "Llama-3.1-70B @ 4k Context (Enterprise Server)"),
        ("70b", 32768, 0.5, 64, "Llama-3.1-70B @ 32k Context (Enterprise Server)"),
        ("70b", 131072, 0.0, 64, "Llama-3.1-70B @ 128k Full Context (0% Prefix Elision)"),
        ("70b", 131072, 0.5, 64, "Llama-3.1-70B @ 128k Full Context (50% Prefix Elision)"),
        ("70b", 131072, 0.8, 64, "Llama-3.1-70B @ 128k Full Context (80% Prefix Elision)"),
    ]

    results = []

    print(f"{'Scenario':<42} | {'Prism WS':<10} | {'Non-Prism WS':<12} | {'Budget':<8} | {'KV Save':<8} | {'Status':<14}")
    print("-" * 105)

    for model, ctx_len, prefix_ratio, budget_gb, label in scenarios:
        prefix_tokens = int(ctx_len * prefix_ratio)
        res = run_ai_compare(model, ctx_len, prefix_tokens, budget_gb)
        if not res:
            continue

        ws = res["working_set"]
        prism_gb = ws["total_prism_working_set_bytes"] / (1024**3)
        non_prism_gb = ws["total_non_prism_working_set_bytes"] / (1024**3)
        budget = budget_gb
        kv_savings = f"{res['kv_cache_savings_pct']:.1f}%"
        status = "CONTAINED" if ws["prism_contained"] else "EXCEEDED"
        if ws["non_prism_swap_thrashing_risk"] and ws["prism_contained"]:
            status += " (SWAP RISK NON-PRISM)"

        print(f"{label:<42} | {prism_gb:>8.2f} GB | {non_prism_gb:>10.2f} GB | {budget:>6} GB | {kv_savings:>8} | {status}")
        results.append({
            "scenario": label,
            "model": res["model"],
            "context_length": ctx_len,
            "prefix_tokens": prefix_tokens,
            "prefix_ratio": prefix_ratio,
            "memory_budget_gb": budget_gb,
            "prism_working_set_gb": round(prism_gb, 3),
            "non_prism_working_set_gb": round(non_prism_gb, 3),
            "kv_cache_savings_pct": res["kv_cache_savings_pct"],
            "dram_traffic_reduction_pct": res["dram_traffic_reduction_pct"],
            "prism_dram_bytes_per_token": res.get("prism_dram_bytes_per_token"),
            "non_prism_dram_bytes_per_token": res.get("non_prism_dram_bytes_per_token"),
            "router_dispatch_latency_ns": res.get("router_dispatch_latency_ns"),
            "non_prism_router_dispatch_latency_ns": res.get("non_prism_router_dispatch_latency_ns"),
            "router_throughput_ops_per_sec": res.get("router_throughput_ops_per_sec"),
            "non_prism_router_throughput_ops_per_sec": res.get("non_prism_router_throughput_ops_per_sec"),
            "prism_contained": ws["prism_contained"],
            "non_prism_swap_thrashing_risk": ws["non_prism_swap_thrashing_risk"],
            "scalability_verdict": res["scalability_verdict"],
            "execution_latency_ms": res.get("execution_latency_ms"),
        })

    print("-" * 105)
    print()

    out_file = Path("target/ai-scaling-comparison.json")
    out_file.parent.mkdir(parents=True, exist_ok=True)
    out_file.write_text(json.dumps(results, indent=2))
    print(f"Detailed AI scaling comparison report written to {out_file}")


if __name__ == "__main__":
    main()
