#![forbid(unsafe_code)]
#![allow(clippy::float_cmp, clippy::unreadable_literal, clippy::cast_lossless)]

use hologram_live::{evaluate_ai_operations_comparison, ModelSpec};

#[test]
fn test_llama_family_specs() {
    let models = [
        (
            ModelSpec::llama3_1b(),
            "Llama-3.2-1B",
            1_230_000_000u64,
            16u32,
            2048u32,
            32u32,
            8u32,
            64u32,
        ),
        (
            ModelSpec::llama3_3b(),
            "Llama-3.2-3B",
            3_210_000_000u64,
            28u32,
            3072u32,
            24u32,
            8u32,
            128u32,
        ),
        (
            ModelSpec::llama2_7b(),
            "Llama-2-7B",
            6_740_000_000u64,
            32u32,
            4096u32,
            32u32,
            32u32,
            128u32,
        ),
        (
            ModelSpec::llama3_8b(),
            "Llama-3.1-8B",
            8_030_000_000u64,
            32u32,
            4096u32,
            32u32,
            8u32,
            128u32,
        ),
        (
            ModelSpec::llama2_13b(),
            "Llama-2-13B",
            13_000_000_000u64,
            40u32,
            5120u32,
            40u32,
            40u32,
            128u32,
        ),
        (
            ModelSpec::llama3_70b(),
            "Llama-3.1-70B",
            70_600_000_000u64,
            80u32,
            8192u32,
            64u32,
            8u32,
            128u32,
        ),
    ];

    for (spec, name, params, layers, hidden, attn_heads, kv_heads, head_dim) in models {
        assert_eq!(spec.name, name);
        assert_eq!(spec.parameter_count, params);
        assert_eq!(spec.layers, layers);
        assert_eq!(spec.hidden_dim, hidden);
        assert_eq!(spec.attention_heads, attn_heads);
        assert_eq!(spec.kv_heads, kv_heads);
        assert_eq!(spec.head_dim, head_dim);

        // Verify KV cache calculation: 2 (K+V) * layers * kv_heads * head_dim * bytes_per_element
        let bytes_per_tok = spec.kv_bytes_per_token(2);
        let expected_bytes = 2 * (layers as u64) * (kv_heads as u64) * (head_dim as u64) * 2;
        assert_eq!(bytes_per_tok, expected_bytes);
    }
}

#[test]
fn test_8b_model_full_context_window_containment() {
    let spec = ModelSpec::llama3_8b();
    let full_128k = 131_072u64;
    let prefix_64k = 65_536u64; // 50% prefix elision
    let budget_16gb = 16u64; // 16 GB developer machine / edge workstation

    let comp = evaluate_ai_operations_comparison(spec, full_128k, prefix_64k, budget_16gb);

    // KV Cache Savings: 50%
    assert_eq!(comp.kv_cache_savings_pct, 50.0);
    assert_eq!(comp.dram_traffic_reduction_pct, 75.0);

    // Un-elided KV cache is 16 GB
    assert_eq!(comp.working_set.ws2_unelided_kv_bytes, 17_179_869_184);
    // Elided KV cache is 8 GB
    assert_eq!(comp.working_set.ws2_kv_cache_bytes, 8_589_934_592);

    // WS-1: 4-bit weights ~ 4.015 GB
    assert_eq!(comp.working_set.ws1_weights_bytes, 4_015_000_000);

    // PrismPM total working set is ~12.62 GB <= 16 GB (Contained!)
    assert!(comp.working_set.prism_contained);
    assert_eq!(
        comp.working_set.total_prism_working_set_bytes,
        12_621_711_808
    );
    assert!(comp.working_set.total_prism_working_set_bytes <= comp.memory_budget_bytes);

    // Non-PrismPM total working set is ~21.26 GB > 16 GB (Swap Thrashing Risk!)
    assert!(comp.working_set.non_prism_swap_thrashing_risk);
    assert_eq!(
        comp.working_set.total_non_prism_working_set_bytes,
        21_261_978_048
    );
    assert!(comp.working_set.total_non_prism_working_set_bytes > comp.memory_budget_bytes);

    assert_eq!(
        comp.scalability_verdict,
        "PrismPM scales to full context window within budget; Non-PrismPM collapses from OS swap thrashing"
    );
}

#[test]
fn test_70b_model_context_scaling_and_prefix_elision() {
    let spec = ModelSpec::llama3_70b();
    let full_128k = 131_072u64;
    let prefix_80pct = 104_857u64; // ~80% prefix elision
    let budget_64gb = 64u64; // 64 GB workstation

    let comp = evaluate_ai_operations_comparison(spec, full_128k, prefix_80pct, budget_64gb);

    // WS-1: 4-bit weights = 35.3 GB
    assert_eq!(comp.working_set.ws1_weights_bytes, 35_300_000_000);

    // Un-elided KV cache: 131,072 * 327,680 bytes = 42.95 GB
    // Non-Prism total = 35.3 GB + 42.95 GB + unfused activations = 78.38 GB > 64 GB budget!
    assert!(comp.working_set.non_prism_swap_thrashing_risk);

    // Prism with 80% prefix elision: KV cache = 8.59 GB
    // Prism total = 35.3 GB + 8.59 GB + fused activations = 43.92 GB <= 64 GB budget!
    assert!(comp.working_set.prism_contained);
    assert!(comp.working_set.total_prism_working_set_bytes <= comp.memory_budget_bytes);
}

#[test]
fn test_1b_and_3b_edge_device_scaling() {
    let m1b = ModelSpec::llama3_1b();
    let m3b = ModelSpec::llama3_3b();

    // 1B model at 32k context on an 8 GB edge device
    let comp_1b = evaluate_ai_operations_comparison(m1b, 32_768, 16_384, 8);
    assert!(comp_1b.working_set.prism_contained);
    assert_eq!(comp_1b.kv_cache_savings_pct, 50.0);

    // 3B model at 32k context on an 8 GB edge device
    let comp_3b = evaluate_ai_operations_comparison(m3b, 32_768, 26_214, 8);
    assert!(comp_3b.working_set.prism_contained);
    assert_eq!(comp_3b.kv_cache_savings_pct, 80.0);
}

#[test]
fn test_arbitrary_components_elimination_record() {
    let spec = ModelSpec::llama3_8b();
    let comp = evaluate_ai_operations_comparison(spec, 4096, 2048, 16);

    let eliminated = &comp.arbitrary_components_eliminated;
    assert_eq!(eliminated.len(), 6);
    assert!(eliminated
        .iter()
        .any(|item| item.contains("Unbounded dynamic KV-cache")));
    assert!(eliminated
        .iter()
        .any(|item| item.contains("OS swap thrashing")));
    assert!(eliminated
        .iter()
        .any(|item| item.contains("Dynamic string-matching")));
    assert!(eliminated
        .iter()
        .any(|item| item.contains("Un-fused DRAM round-trips")));
    assert!(eliminated
        .iter()
        .any(|item| item.contains("Ad-hoc thread and MPSC")));
    assert!(eliminated
        .iter()
        .any(|item| item.contains("quadratic context blowup")));
}

#[test]
fn test_scaling_dimensions_bandwidth_latency_throughput() {
    let spec = ModelSpec::llama3_8b();
    let comp = evaluate_ai_operations_comparison(spec, 131_072, 65_536, 16);

    // Bandwidth scaling
    assert_eq!(comp.prism_dram_bytes_per_token, 4096 * 4);
    assert_eq!(comp.non_prism_dram_bytes_per_token, 4096 * 16);
    assert_eq!(comp.dram_traffic_reduction_pct, 75.0);

    // Latency scaling
    assert_eq!(comp.router_dispatch_latency_ns, 13.2);
    assert_eq!(comp.non_prism_router_dispatch_latency_ns, 520.0);
    assert!(comp.router_dispatch_latency_ns < comp.non_prism_router_dispatch_latency_ns);

    // Throughput scaling
    assert!(comp.router_throughput_ops_per_sec > comp.non_prism_router_throughput_ops_per_sec);
    assert_eq!(comp.router_throughput_ops_per_sec, 75_757_575.0);
    assert_eq!(comp.non_prism_router_throughput_ops_per_sec, 1_923_076.0);
}

#[test]
fn test_alias_resolution_and_overflow_protection() {
    // Aliases
    assert_eq!(
        ModelSpec::from_name("llama-1b"),
        Some(ModelSpec::llama3_1b())
    );
    assert_eq!(
        ModelSpec::from_name("llama-3b"),
        Some(ModelSpec::llama3_3b())
    );
    assert_eq!(ModelSpec::from_name("7b"), Some(ModelSpec::llama2_7b()));
    assert_eq!(
        ModelSpec::from_name("llama-7b"),
        Some(ModelSpec::llama2_7b())
    );
    assert_eq!(
        ModelSpec::from_name("llama2-7b"),
        Some(ModelSpec::llama2_7b())
    );
    assert_eq!(
        ModelSpec::from_name("llama3-7b"),
        Some(ModelSpec::llama2_7b())
    );
    assert_eq!(
        ModelSpec::from_name("llama-8b"),
        Some(ModelSpec::llama3_8b())
    );
    assert_eq!(ModelSpec::from_name("13b"), Some(ModelSpec::llama2_13b()));
    assert_eq!(
        ModelSpec::from_name("llama-13b"),
        Some(ModelSpec::llama2_13b())
    );
    assert_eq!(
        ModelSpec::from_name("llama2-13b"),
        Some(ModelSpec::llama2_13b())
    );
    assert_eq!(
        ModelSpec::from_name("llama-70b"),
        Some(ModelSpec::llama3_70b())
    );
    assert_eq!(ModelSpec::from_name("non-existent"), None);

    // Checked KV bytes
    let m8b = ModelSpec::llama3_8b();
    assert_eq!(m8b.checked_kv_bytes_per_token(2), Some(131_072));
    let m7b = ModelSpec::llama2_7b();
    assert_eq!(m7b.checked_kv_bytes_per_token(2), Some(524_288));
    let m13b = ModelSpec::llama2_13b();
    assert_eq!(m13b.checked_kv_bytes_per_token(2), Some(819_200));

    // Zero context length edge case
    let comp_zero = evaluate_ai_operations_comparison(m8b, 0, 0, 16);
    assert_eq!(comp_zero.kv_cache_savings_pct, 0.0);
    assert_eq!(comp_zero.effective_tokens, 0);
    assert!(comp_zero.working_set.prism_contained);

    // Prefix greater than context length edge case
    let comp_overshoot = evaluate_ai_operations_comparison(m8b, 1000, 2000, 16);
    assert_eq!(comp_overshoot.effective_tokens, 0);
    assert_eq!(comp_overshoot.kv_cache_savings_pct, 100.0);
}

#[test]
fn test_7b_working_set_containment_across_context_lengths() {
    let spec = ModelSpec::llama2_7b();

    // 4k Context (50% prefix elision) on 5 GB budget
    let comp_4k = evaluate_ai_operations_comparison(spec, 4096, 2048, 5);
    assert_eq!(comp_4k.kv_cache_savings_pct, 50.0);
    assert_eq!(comp_4k.working_set.ws1_weights_bytes, 3_370_000_000);
    assert_eq!(comp_4k.working_set.ws2_kv_cache_bytes, 1_073_741_824);
    assert_eq!(comp_4k.working_set.ws2_unelided_kv_bytes, 2_147_483_648);
    assert!(comp_4k.working_set.prism_contained);
    assert!(comp_4k.working_set.non_prism_swap_thrashing_risk);

    // 32k Context (50% prefix elision) on 16 GB budget
    let comp_32k = evaluate_ai_operations_comparison(spec, 32_768, 16_384, 16);
    assert_eq!(comp_32k.kv_cache_savings_pct, 50.0);
    assert_eq!(comp_32k.working_set.ws2_kv_cache_bytes, 8_589_934_592);
    assert_eq!(comp_32k.working_set.ws2_unelided_kv_bytes, 17_179_869_184);
    assert!(comp_32k.working_set.prism_contained);
    assert!(comp_32k.working_set.non_prism_swap_thrashing_risk);
    assert_eq!(
        comp_32k.scalability_verdict,
        "PrismPM scales to full context window within budget; Non-PrismPM collapses from OS swap thrashing"
    );

    // 128k Context (80% prefix elision) on 16 GB budget
    let comp_128k = evaluate_ai_operations_comparison(spec, 131_072, 104_857, 16);
    assert_eq!(comp_128k.kv_cache_savings_pct, 80.0);
    assert!(comp_128k.working_set.prism_contained);
    assert!(comp_128k.working_set.non_prism_swap_thrashing_risk);
}

#[test]
fn test_13b_working_set_containment_across_context_lengths() {
    let spec = ModelSpec::llama2_13b();

    // 4k Context (50% prefix) on 8 GB budget
    let comp_4k = evaluate_ai_operations_comparison(spec, 4096, 2048, 8);
    assert_eq!(comp_4k.kv_cache_savings_pct, 50.0);
    assert_eq!(comp_4k.working_set.ws1_weights_bytes, 6_500_000_000);
    assert_eq!(comp_4k.working_set.ws2_kv_cache_bytes, 1_677_721_600);
    assert_eq!(comp_4k.working_set.ws2_unelided_kv_bytes, 3_355_443_200);
    assert!(comp_4k.working_set.prism_contained);
    assert!(comp_4k.working_set.non_prism_swap_thrashing_risk);

    // 32k Context (50% prefix) on 24 GB budget
    let comp_32k = evaluate_ai_operations_comparison(spec, 32_768, 16_384, 24);
    assert_eq!(comp_32k.kv_cache_savings_pct, 50.0);
    assert!(comp_32k.working_set.prism_contained);
    assert!(comp_32k.working_set.non_prism_swap_thrashing_risk);

    // 128k Context (80% prefix) on 32 GB budget
    let comp_128k = evaluate_ai_operations_comparison(spec, 131_072, 104_857, 32);
    assert_eq!(comp_128k.kv_cache_savings_pct, 80.0);
    assert!(comp_128k.working_set.prism_contained);
    assert!(comp_128k.working_set.non_prism_swap_thrashing_risk);
    assert_eq!(
        comp_128k.scalability_verdict,
        "PrismPM scales to full context window within budget; Non-PrismPM collapses from OS swap thrashing"
    );
}

#[test]
fn test_70b_working_set_containment_across_context_lengths() {
    let spec = ModelSpec::llama3_70b();

    // 4k Context (50% prefix) on 48 GB budget
    let comp_4k = evaluate_ai_operations_comparison(spec, 4096, 2048, 48);
    assert_eq!(comp_4k.kv_cache_savings_pct, 50.0);
    assert_eq!(comp_4k.working_set.ws1_weights_bytes, 35_300_000_000);
    assert_eq!(comp_4k.working_set.ws2_kv_cache_bytes, 671_088_640);
    assert_eq!(comp_4k.working_set.ws2_unelided_kv_bytes, 1_342_177_280);
    assert!(comp_4k.working_set.prism_contained);
    assert!(!comp_4k.working_set.non_prism_swap_thrashing_risk); // Contained on both

    // 32k Context (50% prefix) on 40 GB budget
    let comp_32k = evaluate_ai_operations_comparison(spec, 32_768, 16_384, 40);
    assert_eq!(comp_32k.kv_cache_savings_pct, 50.0);
    assert_eq!(comp_32k.working_set.ws2_kv_cache_bytes, 5_368_709_120);
    assert_eq!(comp_32k.working_set.ws2_unelided_kv_bytes, 10_737_418_240);
    assert!(comp_32k.working_set.prism_contained);
    assert!(comp_32k.working_set.non_prism_swap_thrashing_risk);

    // 128k Context (50% prefix) on 64 GB budget
    let comp_128k = evaluate_ai_operations_comparison(spec, 131_072, 65_536, 64);
    assert_eq!(comp_128k.kv_cache_savings_pct, 50.0);
    assert_eq!(comp_128k.working_set.ws2_kv_cache_bytes, 21_474_836_480);
    assert_eq!(comp_128k.working_set.ws2_unelided_kv_bytes, 42_949_672_960);
    assert!(comp_128k.working_set.prism_contained);
    assert!(comp_128k.working_set.non_prism_swap_thrashing_risk);
}
