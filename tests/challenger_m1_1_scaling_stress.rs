#![forbid(unsafe_code)]
#![allow(clippy::float_cmp, clippy::unreadable_literal, clippy::cast_lossless)]

use hologram_live::{
    evaluate_ai_operations_comparison, kv_effective_tokens, ModelSpec,
};

const CONTEXT_LENGTHS: [u64; 7] = [0, 1, 4096, 32768, 65536, 131072, 262144];
const PREFIX_RATIOS: [f64; 5] = [0.0, 0.50, 0.80, 1.00, 1.50];
const GIB_BYTES: u64 = 1024 * 1024 * 1024;

fn get_all_models() -> [(ModelSpec, &'static str); 6] {
    [
        (ModelSpec::llama3_1b(), "1b"),
        (ModelSpec::llama3_3b(), "3b"),
        (ModelSpec::llama2_7b(), "7b"),
        (ModelSpec::llama3_8b(), "8b"),
        (ModelSpec::llama2_13b(), "13b"),
        (ModelSpec::llama3_70b(), "70b"),
    ]
}

#[test]
fn test_exhaustive_scaling_matrix_210_combinations() {
    let models = get_all_models();
    let mut total_evaluated = 0;

    for (spec, name) in &models {
        // Verify from_name matches preset
        let resolved = ModelSpec::from_name(name).expect("from_name must resolve");
        assert_eq!(*spec, resolved, "Resolved model must match preset for {name}");

        let bytes_per_elem = 2u64;
        let bytes_per_token = spec.kv_bytes_per_token(bytes_per_elem);
        assert_eq!(
            spec.checked_kv_bytes_per_token(bytes_per_elem),
            Some(bytes_per_token),
            "Checked KV bytes per token must match unchecked for {name}"
        );

        let ws1_expected = spec.parameter_count / 2;
        let ws3_fused_expected = u64::from(spec.hidden_dim) * 4 * 1024;
        let ws3_unfused_expected = ws3_fused_expected * 4;

        for &ctx_len in &CONTEXT_LENGTHS {
            for &ratio in &PREFIX_RATIOS {
                let prefix_tokens = (ctx_len as f64 * ratio).round() as u64;
                let effective_tokens = kv_effective_tokens(ctx_len, prefix_tokens);

                // If ratio >= 1.0, effective tokens must be 0
                if ratio >= 1.0 {
                    assert_eq!(
                        effective_tokens, 0,
                        "Effective tokens must be 0 when ratio >= 1.0 ({ratio}) for ctx {ctx_len}"
                    );
                } else if ctx_len > 0 {
                    assert!(
                        effective_tokens <= ctx_len,
                        "Effective tokens must not exceed context length"
                    );
                }

                // Arbitrary budget of 128 GB to inspect raw values
                let comp = evaluate_ai_operations_comparison(
                    *spec,
                    ctx_len,
                    prefix_tokens,
                    128,
                );

                total_evaluated += 1;

                // 1. Verify WS-1 (weights)
                assert_eq!(
                    comp.working_set.ws1_weights_bytes, ws1_expected,
                    "WS-1 weights byte mismatch for {name}"
                );

                // 2. Verify WS-2 (KV-cache)
                let expected_ws2_elided = effective_tokens.saturating_mul(bytes_per_token);
                let expected_ws2_unelided = ctx_len.saturating_mul(bytes_per_token);
                assert_eq!(
                    comp.working_set.ws2_kv_cache_bytes, expected_ws2_elided,
                    "WS-2 elided KV cache mismatch for {name} ctx {ctx_len} ratio {ratio}"
                );
                assert_eq!(
                    comp.working_set.ws2_unelided_kv_bytes, expected_ws2_unelided,
                    "WS-2 unelided KV cache mismatch for {name} ctx {ctx_len}"
                );

                // 3. Verify WS-3 (activations)
                assert_eq!(
                    comp.working_set.ws3_activation_bytes, ws3_fused_expected,
                    "WS-3 fused activation mismatch for {name}"
                );
                assert_eq!(
                    comp.working_set.ws3_unfused_activation_bytes, ws3_unfused_expected,
                    "WS-3 unfused activation mismatch for {name}"
                );

                // 4. Invariant: Prism working set must strictly be <= non-Prism working set
                assert!(
                    comp.working_set.total_prism_working_set_bytes
                        <= comp.working_set.total_non_prism_working_set_bytes,
                    "Prism working set must be <= Non-Prism working set for {name} ctx {ctx_len}"
                );

                // 5. Verify KV savings percentage math
                if ctx_len == 0 {
                    assert_eq!(
                        comp.kv_cache_savings_pct, 0.0,
                        "Zero context length must report 0.0% savings"
                    );
                } else if ratio >= 1.0 {
                    assert_eq!(
                        comp.kv_cache_savings_pct, 100.0,
                        "Ratio >= 1.0 must report 100.0% savings for ctx {ctx_len}"
                    );
                } else if (ratio - 0.50).abs() < f64::EPSILON && ctx_len % 2 == 0 {
                    assert_eq!(
                        comp.kv_cache_savings_pct, 50.0,
                        "50% ratio on even context must report 50.0% savings"
                    );
                } else if (ratio - 0.80).abs() < f64::EPSILON && ctx_len % 5 == 0 {
                    assert_eq!(
                        comp.kv_cache_savings_pct, 80.0,
                        "80% ratio on divisible context must report 80.0% savings"
                    );
                } else if (ratio - 0.0).abs() < f64::EPSILON {
                    assert_eq!(
                        comp.kv_cache_savings_pct, 0.0,
                        "0% ratio must report 0.0% savings"
                    );
                }
            }
        }
    }

    assert_eq!(
        total_evaluated, 210,
        "Total evaluated configurations must equal exactly 6 * 7 * 5 = 210"
    );
}

#[test]
fn test_exact_memory_budget_threshold_flip() {
    let models = get_all_models();

    for (spec, name) in &models {
        for &ctx_len in &[4096u64, 32768u64, 131072u64] {
            for &ratio in &[0.50f64, 0.80f64] {
                let prefix_tokens = (ctx_len as f64 * ratio).round() as u64;
                let sample = evaluate_ai_operations_comparison(*spec, ctx_len, prefix_tokens, 128);

                let prism_bytes = sample.working_set.total_prism_working_set_bytes;
                let non_prism_bytes = sample.working_set.total_non_prism_working_set_bytes;

                assert!(
                    prism_bytes < non_prism_bytes,
                    "Prism bytes must be strictly less than non-Prism bytes when prefix > 0"
                );

                // Compute exact ceiling in GB for Prism and floor/ceiling for Non-Prism
                let min_gb_for_prism = (prism_bytes + GIB_BYTES - 1) / GIB_BYTES;
                let max_gb_thrashing = (non_prism_bytes - 1) / GIB_BYTES;

                // 1. At budget below min_gb_for_prism, Prism is NOT contained
                if min_gb_for_prism > 1 {
                    let sub_budget = min_gb_for_prism - 1;
                    let comp_sub = evaluate_ai_operations_comparison(
                        *spec,
                        ctx_len,
                        prefix_tokens,
                        sub_budget,
                    );
                    assert!(
                        !comp_sub.working_set.prism_contained,
                        "Prism must NOT be contained at {sub_budget} GB for {name} (requires {min_gb_for_prism} GB)"
                    );
                    assert!(
                        comp_sub.working_set.non_prism_swap_thrashing_risk,
                        "Non-Prism must risk thrashing when budget is below Prism requirement"
                    );
                }

                // 2. In the containment window [min_gb_for_prism, max_gb_thrashing]:
                // PrismPM is strictly contained, while Non-PrismPM risks swap thrashing!
                if min_gb_for_prism <= max_gb_thrashing {
                    for test_gb in [min_gb_for_prism, max_gb_thrashing] {
                        let comp = evaluate_ai_operations_comparison(
                            *spec,
                            ctx_len,
                            prefix_tokens,
                            test_gb,
                        );
                        assert!(
                            comp.working_set.prism_contained,
                            "Prism MUST be contained at {test_gb} GB for {name} ctx {ctx_len}"
                        );
                        assert!(
                            comp.working_set.non_prism_swap_thrashing_risk,
                            "Non-Prism MUST risk swap thrashing at {test_gb} GB for {name} ctx {ctx_len}"
                        );
                        assert_eq!(
                            comp.scalability_verdict,
                            "PrismPM scales to full context window within budget; Non-PrismPM collapses from OS swap thrashing"
                        );
                    }
                }

                // 3. At budget above non_prism_bytes ceiling, thrashing risk flips to false
                let safe_gb = (non_prism_bytes + GIB_BYTES - 1) / GIB_BYTES + 1;
                let comp_safe = evaluate_ai_operations_comparison(
                    *spec,
                    ctx_len,
                    prefix_tokens,
                    safe_gb,
                );
                assert!(
                    comp_safe.working_set.prism_contained,
                    "Prism must be contained at safe budget {safe_gb} GB"
                );
                assert!(
                    !comp_safe.working_set.non_prism_swap_thrashing_risk,
                    "Non-Prism thrashing risk must flip to FALSE at {safe_gb} GB for {name}"
                );
                assert_eq!(
                    comp_safe.scalability_verdict,
                    "Contained on both; PrismPM delivers 75% DRAM reduction and prefix KV elision"
                );
            }
        }
    }
}

#[test]
fn test_llama2_mha_vs_llama3_gqa_divergence() {
    let spec_7b = ModelSpec::llama2_7b();
    let spec_8b = ModelSpec::llama3_8b();

    // 7B uses MHA (32 KV heads), 8B uses GQA (8 KV heads)
    assert_eq!(spec_7b.kv_heads, 32);
    assert_eq!(spec_8b.kv_heads, 8);

    // KV footprint per token: 7B is 4x larger than 8B!
    let rate_7b = spec_7b.kv_bytes_per_token(2);
    let rate_8b = spec_8b.kv_bytes_per_token(2);
    assert_eq!(rate_7b, 524_288); // 512 KiB/token
    assert_eq!(rate_8b, 131_072); // 128 KiB/token
    assert_eq!(rate_7b, rate_8b * 4);

    // At 256k context window:
    let ctx_256k = 262_144u64;
    let unelided_7b_bytes = ctx_256k * rate_7b;
    let unelided_8b_bytes = ctx_256k * rate_8b;

    // 7B unelided KV cache is 137.4 GB!
    assert_eq!(unelided_7b_bytes, 137_438_953_472);
    // 8B unelided KV cache is 34.36 GB
    assert_eq!(unelided_8b_bytes, 34_359_738_368);

    // Under 80% prefix elision on a 32 GB budget:
    // 7B elided KV cache = 262_144 * 0.2 * 524_288 = 27.49 GB
    // Total 7B Prism WS = 3.37 GB (weights) + 27.49 GB (KV) + 0.017 GB (act) = ~30.87 GB <= 32 GB (CONTAINED!)
    let prefix_80pct = (ctx_256k as f64 * 0.8).round() as u64;
    let comp_7b = evaluate_ai_operations_comparison(spec_7b, ctx_256k, prefix_80pct, 32);
    assert!(
        comp_7b.working_set.prism_contained,
        "7B with 80% prefix elision must be contained in 32 GB workstation"
    );
    assert!(
        comp_7b.working_set.non_prism_swap_thrashing_risk,
        "7B unelided must trigger catastrophic swap thrashing (137 GB > 32 GB)"
    );

    // Compare 13B (MHA: 40 KV heads) vs 70B (GQA: 8 KV heads)
    let spec_13b = ModelSpec::llama2_13b();
    let spec_70b = ModelSpec::llama3_70b();
    let rate_13b = spec_13b.kv_bytes_per_token(2);
    let rate_70b = spec_70b.kv_bytes_per_token(2);

    assert_eq!(rate_13b, 819_200); // 800 KiB/token
    assert_eq!(rate_70b, 327_680); // 320 KiB/token
    assert!(
        rate_13b > rate_70b,
        "13B KV rate per token must be 2.5x larger than 70B due to MHA vs GQA"
    );
    assert_eq!(rate_13b * 2, rate_70b * 5); // 800 * 2 = 1600, 320 * 5 = 1600
}

#[test]
fn test_boundary_zero_and_single_token() {
    let models = get_all_models();

    for (spec, _name) in &models {
        // Budget scaled to accommodate base weights (4-bit) + activation envelope + 1 GiB headroom
        let budget_gb = (spec.parameter_count / 2 + GIB_BYTES - 1) / GIB_BYTES + 1;

        // Zero context length
        let zero_comp = evaluate_ai_operations_comparison(*spec, 0, 0, budget_gb);
        assert_eq!(zero_comp.effective_tokens, 0);
        assert_eq!(zero_comp.kv_cache_savings_pct, 0.0);
        assert_eq!(zero_comp.working_set.ws2_kv_cache_bytes, 0);
        assert_eq!(zero_comp.working_set.ws2_unelided_kv_bytes, 0);
        assert!(zero_comp.working_set.prism_contained);

        // Single token context length
        let one_no_prefix = evaluate_ai_operations_comparison(*spec, 1, 0, budget_gb);
        assert_eq!(one_no_prefix.effective_tokens, 1);
        assert_eq!(one_no_prefix.kv_cache_savings_pct, 0.0);
        assert_eq!(
            one_no_prefix.working_set.ws2_kv_cache_bytes,
            spec.kv_bytes_per_token(2)
        );

        let one_full_prefix = evaluate_ai_operations_comparison(*spec, 1, 1, budget_gb);
        assert_eq!(one_full_prefix.effective_tokens, 0);
        assert_eq!(one_full_prefix.kv_cache_savings_pct, 100.0);
        assert_eq!(one_full_prefix.working_set.ws2_kv_cache_bytes, 0);

        let one_overshoot = evaluate_ai_operations_comparison(*spec, 1, 5, budget_gb);
        assert_eq!(one_overshoot.effective_tokens, 0);
        assert_eq!(one_overshoot.kv_cache_savings_pct, 100.0);
        assert_eq!(one_overshoot.working_set.ws2_kv_cache_bytes, 0);
    }
}

#[test]
fn test_extreme_memory_budgets_zero_and_saturation() {
    let spec = ModelSpec::llama2_7b();

    // Budget = 0 GB: Prism is NOT contained, Non-Prism risks thrashing
    let zero_budget = evaluate_ai_operations_comparison(spec, 4096, 2048, 0);
    assert!(!zero_budget.working_set.prism_contained);
    assert!(zero_budget.working_set.non_prism_swap_thrashing_risk);
    assert_eq!(
        zero_budget.scalability_verdict,
        "Exceeds edge memory budget; scale budget or enable deeper quantization"
    );

    // Extreme budget = u64::MAX / GIB_BYTES
    let max_gb = u64::MAX / GIB_BYTES;
    let max_budget = evaluate_ai_operations_comparison(spec, 4096, 2048, max_gb);
    assert!(max_budget.working_set.prism_contained);
    assert!(!max_budget.working_set.non_prism_swap_thrashing_risk);
    assert_eq!(
        max_budget.scalability_verdict,
        "Contained on both; PrismPM delivers 75% DRAM reduction and prefix KV elision"
    );
}
