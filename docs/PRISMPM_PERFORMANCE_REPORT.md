# PrismPM Performance Comparison Report: Hologram Live

This document links to the authoritative performance and capabilities comparison reports:

- [docs/HOLOGRAM_AI_OPERATIONS_AND_CAPABILITIES.md](HOLOGRAM_AI_OPERATIONS_AND_CAPABILITIES.md): Comprehensive operations, capabilities, and scaling report for `hologram-ai` under PrismPM vs non-PrismPM, demonstrating working set containment ($WS-1$..$WS-3$), $75\%$ DRAM traffic reduction (`FU-1`–`FU-4`), and full $128\text{k}$ context window scaling.
- [docs/superpowers/specs/2026-09-28-prismpm-performance-comparison.md](superpowers/specs/2026-09-28-prismpm-performance-comparison.md): Complete system-wide benchmark suite, empirical CLI latency measurements, UOR inference cost model analysis, and cluster deployment verification.

## Quick Summary

- **Command Router Throughput:** $75,854,223\text{ ops/sec}$ ($13.2\text{ ns/op}$) in optimized release mode.
- **Inference DRAM Traffic Savings:** $75.0\%$ memory bandwidth reduction via 4-way fused kernel execution (`FU-1`–`FU-4`).
- **KV-Cache Memory Savings:** $50.0\%$ to $80.0\%$ reduction in memory footprint through formal prefix token elision (`kv_effective_tokens`).
- **Peak RSS Footprint:** $11.99\text{ MB}$ (doctor) / $7.32\text{ MB}$ (help) / $12.06\text{ MB}$ (status) with flat memory scaling ($\Delta < 1.0\%$).
- **Cluster Deployment Verification:** $65.56\text{ ms}$ Docker Compose spec validation, $0.59\text{ ms}$ Kubernetes 47-resource reconciliation.
- **Verification Gates:** $100\%$ green across `prismpm check`, `prismpm verify`, `lexlean verify`, `cargo test --test cluster_e2e_prism`, and `cargo test --test prism_conformance`.
