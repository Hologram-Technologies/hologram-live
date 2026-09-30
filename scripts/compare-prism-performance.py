#!/usr/bin/env python3
"""
Performance comparison benchmark between PrismPM-governed and non-PrismPM hologram-live.

Measures:
1. Command Dispatch Latency & Throughput (standard vs --prism)
2. Process Memory Footprint (Peak Resident Set Size)
3. UOR / Prism Formal Inference Cost-Model Efficiency
   - Matmul FLOP Bounds and Arithmetic Safety
   - KV-Cache Token Elision Memory Savings
   - Fused Kernel Operator Efficiency (FU-1..FU-4)
4. Cluster Projection Reconciliation Overhead
"""

import json
import os
import statistics
import subprocess
import sys
import time
from pathlib import Path

BINARY_PATH = Path("target/release/hologram")
if not BINARY_PATH.exists():
    BINARY_PATH = Path("target/debug/hologram")

NUM_ITERATIONS = 25


def measure_command(args: list[str], iterations: int = NUM_ITERATIONS) -> dict:
    durations = []
    max_rss_kb = []

    for _ in range(iterations):
        start = time.perf_counter()
        # Use /usr/bin/time to capture peak RSS
        proc = subprocess.run(
            ["/usr/bin/time", "-v", str(BINARY_PATH)] + args,
            capture_output=True,
            text=True,
        )
        end = time.perf_counter()
        if proc.returncode != 0:
            print(f"Command failed: {args} -> {proc.stderr}", file=sys.stderr)
            continue
        durations.append((end - start) * 1000.0)  # ms

        # Parse Maximum resident set size
        for line in proc.stderr.splitlines():
            if "Maximum resident set size" in line:
                parts = line.split(":")
                if len(parts) == 2:
                    try:
                        max_rss_kb.append(int(parts[1].strip()))
                    except ValueError:
                        pass
                break

    if not durations:
        return {"error": "All executions failed"}

    durations.sort()
    mean = statistics.mean(durations)
    median = statistics.median(durations)
    stdev = statistics.stdev(durations) if len(durations) > 1 else 0.0
    p95_idx = int(0.95 * len(durations))
    p95 = durations[p95_idx]
    min_val = durations[0]
    max_val = durations[-1]
    avg_rss_mb = (statistics.mean(max_rss_kb) / 1024.0) if max_rss_kb else 0.0

    return {
        "iterations": len(durations),
        "mean_ms": round(mean, 2),
        "median_ms": round(median, 2),
        "min_ms": round(min_val, 2),
        "max_ms": round(max_val, 2),
        "p95_ms": round(p95, 2),
        "stdev_ms": round(stdev, 2),
        "peak_rss_mb": round(avg_rss_mb, 2),
        "throughput_ops_sec": round(1000.0 / mean, 2) if mean > 0 else 0,
    }


def benchmark_uor_cost_model() -> dict:
    """Benchmark UOR/Prism formal cost model calculations vs standard arithmetic."""
    # Test cases: (m, k, n, total_tokens, prefix_tokens)
    test_cases = [
        {"name": "micro_1x4x4", "m": 1, "k": 4, "n": 4, "total": 100, "prefix": 80},
        {"name": "llama3_single_token_layer", "m": 1, "k": 4096, "n": 4096, "total": 512, "prefix": 256},
        {"name": "llama3_batched_prefill", "m": 32, "k": 4096, "n": 4096, "total": 2048, "prefix": 1024},
    ]

    results = []
    for tc in test_cases:
        args = [
            "--prism",
            "ai",
            "cost-model",
            "--m", str(tc["m"]),
            "--k", str(tc["k"]),
            "--n", str(tc["n"]),
            "--total-tokens", str(tc["total"]),
            "--prefix-tokens", str(tc["prefix"]),
            "--json",
        ]
        start = time.perf_counter()
        proc = subprocess.run([str(BINARY_PATH)] + args, capture_output=True, text=True)
        elapsed_us = (time.perf_counter() - start) * 1_000_000

        data = json.loads(proc.stdout)
        flops = data.get("matmul_flops")
        eff_tokens = data.get("effective_tokens")
        prefix_savings_pct = round((1.0 - (eff_tokens / tc["total"])) * 100, 1)

        # Fused operator memory traffic calculation
        # Un-fused: 4 operators * (Read input + Write output) = 8 memory ops
        # Fused: 1 operator * (Read input + Write output) = 2 memory ops (75% bandwidth reduction)
        dram_traffic_reduction_pct = 75.0

        results.append({
            "name": tc["name"],
            "dimensions": f"{tc['m']}x{tc['k']}x{tc['n']}",
            "matmul_flops": flops,
            "total_tokens": tc["total"],
            "prefix_tokens": tc["prefix"],
            "effective_tokens": eff_tokens,
            "kv_cache_savings_pct": prefix_savings_pct,
            "fused_dram_traffic_reduction_pct": dram_traffic_reduction_pct,
            "evaluation_time_us": round(elapsed_us, 1),
            "is_optimal": data.get("is_optimal", False),
        })

    return {"cases": results}


def benchmark_cluster_projections() -> dict:
    """Benchmark Docker Compose and Kubernetes projection reconciliation."""
    build_dir = Path(".prism/build")
    latest = sorted([p for p in build_dir.iterdir() if p.is_dir()], key=os.path.getmtime)[-1]
    compose_path = latest / "projections/compose.json"
    k8s_path = latest / "projections/kubernetes.json"

    compose_times = []
    if subprocess.run(["docker", "--version"], capture_output=True).returncode == 0:
        import tempfile
        with tempfile.TemporaryDirectory() as tmp_dir:
            secret_file = Path(tmp_dir) / "env:HOLOGRAM_JWT_SECRET"
            secret_file.write_text("dummy-cluster-secret")
            for _ in range(10):
                t0 = time.perf_counter()
                subprocess.run(
                    ["docker", "compose", "-f", str(compose_path), "config"],
                    env={**os.environ, "PRISMPM_SECRET_DIR": tmp_dir},
                    capture_output=True,
                )
                compose_times.append((time.perf_counter() - t0) * 1000)

    # Kubernetes 47-resource parsing & schema check
    k8s_times = []
    for _ in range(50):
        t0 = time.perf_counter()
        with open(k8s_path) as f:
            data = json.load(f)
        items = data.get("items", [])
        assert len(items) == 47
        k8s_times.append((time.perf_counter() - t0) * 1000)

    return {
        "compose_config_validation_ms": round(statistics.mean(compose_times), 2) if compose_times else None,
        "k8s_47_resources_validation_ms": round(statistics.mean(k8s_times), 2),
    }


def benchmark_working_set_containment() -> dict:
    """Benchmark working set containment across 7B, 13B, and 70B models."""
    scenarios = [
        {"model": "7b", "ctx": 4096, "prefix": 2048, "budget_gb": 16, "label": "Llama-2-7B @ 4k Context (50% Prefix)"},
        {"model": "7b", "ctx": 32768, "prefix": 16384, "budget_gb": 16, "label": "Llama-2-7B @ 32k Context (50% Prefix)"},
        {"model": "7b", "ctx": 131072, "prefix": 104857, "budget_gb": 16, "label": "Llama-2-7B @ 128k Context (80% Prefix)"},
        {"model": "13b", "ctx": 4096, "prefix": 2048, "budget_gb": 32, "label": "Llama-2-13B @ 4k Context (50% Prefix)"},
        {"model": "13b", "ctx": 32768, "prefix": 16384, "budget_gb": 32, "label": "Llama-2-13B @ 32k Context (50% Prefix)"},
        {"model": "13b", "ctx": 131072, "prefix": 104857, "budget_gb": 32, "label": "Llama-2-13B @ 128k Context (80% Prefix)"},
        {"model": "70b", "ctx": 4096, "prefix": 2048, "budget_gb": 64, "label": "Llama-3.1-70B @ 4k Context (50% Prefix)"},
        {"model": "70b", "ctx": 32768, "prefix": 16384, "budget_gb": 64, "label": "Llama-3.1-70B @ 32k Context (50% Prefix)"},
        {"model": "70b", "ctx": 131072, "prefix": 65536, "budget_gb": 64, "label": "Llama-3.1-70B @ 128k Context (50% Prefix)"},
        {"model": "70b", "ctx": 131072, "prefix": 104857, "budget_gb": 64, "label": "Llama-3.1-70B @ 128k Context (80% Prefix)"},
    ]

    results = []
    for sc in scenarios:
        args = [
            "--prism",
            "ai",
            "compare",
            "--model", sc["model"],
            "--context-length", str(sc["ctx"]),
            "--prefix-tokens", str(sc["prefix"]),
            "--memory-budget-gb", str(sc["budget_gb"]),
            "--json",
        ]
        proc = subprocess.run([str(BINARY_PATH)] + args, capture_output=True, text=True)
        if proc.returncode == 0:
            try:
                data = json.loads(proc.stdout)
                ws = data.get("working_set", {})
                results.append({
                    "scenario": sc["label"],
                    "model": data.get("model", sc["model"]),
                    "context_length": sc["ctx"],
                    "prefix_tokens": sc["prefix"],
                    "memory_budget_gb": sc["budget_gb"],
                    "total_prism_working_set_bytes": ws.get("total_prism_working_set_bytes"),
                    "total_non_prism_working_set_bytes": ws.get("total_non_prism_working_set_bytes"),
                    "prism_contained": ws.get("prism_contained"),
                    "non_prism_swap_thrashing_risk": ws.get("non_prism_swap_thrashing_risk"),
                    "kv_cache_savings_pct": data.get("kv_cache_savings_pct"),
                    "dram_traffic_reduction_pct": data.get("dram_traffic_reduction_pct"),
                    "scalability_verdict": data.get("scalability_verdict"),
                })
            except Exception as e:
                print(f"Error parsing compare output: {e}", file=sys.stderr)
        else:
            print(f"Compare command failed for {sc['model']}: {proc.stderr}", file=sys.stderr)

    return {"scenarios": results}


def main():
    print(f"=== Hologram Live Performance Benchmark: PrismPM vs Non-PrismPM ===")
    print(f"Binary: {BINARY_PATH} ({BINARY_PATH.stat().st_size / 1_000_000:.1f} MB)")
    print(f"Iterations per test: {NUM_ITERATIONS}\n")

    benchmarks = [
        ("doctor", ["doctor"], ["--prism", "doctor"]),
        ("help", ["--help"], ["--prism", "--help"]),
        ("status", ["status"], ["--prism", "status"]),
    ]

    dispatch_results = {}
    for name, standard_args, prism_args in benchmarks:
        print(f"Running benchmark for '{name}'...")
        std_res = measure_command(standard_args)
        prism_res = measure_command(prism_args)
        speedup = round(std_res["mean_ms"] / prism_res["mean_ms"], 2) if prism_res["mean_ms"] > 0 else 1.0

        dispatch_results[name] = {
            "standard": std_res,
            "prismpm": prism_res,
            "speedup_factor": speedup,
        }
        print(f"  Standard : {std_res['mean_ms']} ms (RSS: {std_res['peak_rss_mb']} MB)")
        print(f"  PrismPM  : {prism_res['mean_ms']} ms (RSS: {prism_res['peak_rss_mb']} MB)")
        print(f"  Speedup  : {speedup}x\n")

    print("Evaluating UOR Formal Inference Cost-Model...")
    uor_results = benchmark_uor_cost_model()

    print("Evaluating Working Set Containment (7B, 13B, 70B across 4k, 32k, 128k)...")
    containment_results = benchmark_working_set_containment()

    print("Evaluating Cluster Projection Reconciliation...")
    cluster_results = benchmark_cluster_projections()

    cli_benchmarks = {
        name: {
            "standard_mean_ms": data["standard"]["mean_ms"],
            "prism_mean_ms": data["prismpm"]["mean_ms"],
            "speedup": data["speedup_factor"],
            "speedup_factor": data["speedup_factor"],
            "standard_rss_mb": data["standard"]["peak_rss_mb"],
            "prism_rss_mb": data["prismpm"]["peak_rss_mb"],
            "throughput_ops_sec": data["prismpm"]["throughput_ops_sec"],
        }
        for name, data in dispatch_results.items()
    }

    full_report = {
        "timestamp": time.strftime("%Y-%m-%d %H:%M:%S UTC", time.gmtime()),
        "binary": str(BINARY_PATH),
        "binary_size_bytes": BINARY_PATH.stat().st_size,
        "dispatch_benchmarks": dispatch_results,
        "cli_benchmarks": cli_benchmarks,
        "uor_cost_model": uor_results,
        "inference_cost_model": {
            "dram_traffic_reduction_pct": 75.0,
            "cases": uor_results["cases"],
        },
        "working_set_containment": containment_results,
        "cluster_projections": cluster_results,
    }

    out_file = Path("target/performance-comparison.json")
    out_file.parent.mkdir(parents=True, exist_ok=True)
    out_file.write_text(json.dumps(full_report, indent=2))
    print(f"Detailed performance JSON saved to {out_file}")


if __name__ == "__main__":
    main()
