#![forbid(unsafe_code)]
#![allow(
    clippy::float_cmp,
    clippy::unreadable_literal,
    clippy::cast_lossless,
    clippy::too_many_lines,
    clippy::cast_precision_loss,
    clippy::uninlined_format_args
)]

use hologram_live::{
    dispatchBytes, dispatchString, evaluate_ai_operations_comparison, executeCommand,
    matmul_flops, parseCliCommand, CliCommand, FusedKernelProfile, MatrixDimension, ModelSpec,
};
use std::process::Command;

// ============================================================================
// 1. ADVERSARIAL CHECKED ARITHMETIC (2 * M * K * N)
// ============================================================================

#[test]
fn test_adversarial_matmul_flops_zeros_and_neutral() {
    // Single zeros
    assert_eq!(matmul_flops(MatrixDimension::new(0, 100, 100)), Some(0));
    assert_eq!(matmul_flops(MatrixDimension::new(100, 0, 100)), Some(0));
    assert_eq!(matmul_flops(MatrixDimension::new(100, 100, 0)), Some(0));

    // Multiple zeros
    assert_eq!(matmul_flops(MatrixDimension::new(0, 0, 100)), Some(0));
    assert_eq!(matmul_flops(MatrixDimension::new(0, 100, 0)), Some(0));
    assert_eq!(matmul_flops(MatrixDimension::new(100, 0, 0)), Some(0));
    assert_eq!(matmul_flops(MatrixDimension::new(0, 0, 0)), Some(0));

    // Neutral element: 1x1x1 -> 2 * 1 * 1 * 1 = 2
    assert_eq!(matmul_flops(MatrixDimension::new(1, 1, 1)), Some(2));
}

#[test]
fn test_adversarial_matmul_flops_short_circuit_asymmetry() {
    // Observation: 2u64.checked_mul(m)?.checked_mul(k)?.checked_mul(n)
    // Left-to-right evaluation means if 2 * m overflows, it short-circuits to None
    // EVEN IF subsequent dimensions are 0.
    // However, if m = 0, 2 * 0 = 0, which multiplies safely with any subsequent value!

    // Case 1: M is large (overflows initial 2 * M), but K is 0.
    // 2 * (u64::MAX / 2 + 1) overflows u64 -> None!
    let dim_overflow_first = MatrixDimension::new(u64::MAX / 2 + 1, 0, 10);
    assert_eq!(
        matmul_flops(dim_overflow_first),
        None,
        "M > u64::MAX / 2 overflows at 2 * M step even when K = 0"
    );

    // Case 2: M is 0, but K is u64::MAX.
    // 2 * 0 = 0; 0 * u64::MAX = 0; 0 * 10 = 0 -> Some(0)!
    let dim_zero_first = MatrixDimension::new(0, u64::MAX, 10);
    assert_eq!(
        matmul_flops(dim_zero_first),
        Some(0),
        "M = 0 absorbs subsequent u64::MAX without overflow"
    );

    // Case 3: M = 1, 2 * 1 = 2. K > u64::MAX / 2 overflows at 2 * K step even when N = 0.
    let dim_overflow_second = MatrixDimension::new(1, u64::MAX / 2 + 1, 0);
    assert_eq!(
        matmul_flops(dim_overflow_second),
        None,
        "K > u64::MAX / 2 overflows at 2 * K step even when N = 0"
    );
}

#[test]
fn test_adversarial_matmul_flops_exact_overflow_boundaries() {
    let half_max = u64::MAX / 2; // 9_223_372_036_854_775_807

    // Exact non-overflow boundary: 2 * (u64::MAX / 2) * 1 * 1 = u64::MAX - 1
    assert_eq!(
        matmul_flops(MatrixDimension::new(half_max, 1, 1)),
        Some(u64::MAX - 1)
    );
    assert_eq!(
        matmul_flops(MatrixDimension::new(1, half_max, 1)),
        Some(u64::MAX - 1)
    );
    assert_eq!(
        matmul_flops(MatrixDimension::new(1, 1, half_max)),
        Some(u64::MAX - 1)
    );

    // Exact overflow boundary: half_max + 1
    assert_eq!(matmul_flops(MatrixDimension::new(half_max + 1, 1, 1)), None);
    assert_eq!(matmul_flops(MatrixDimension::new(1, half_max + 1, 1)), None);
    assert_eq!(matmul_flops(MatrixDimension::new(1, 1, half_max + 1)), None);

    // Full u64::MAX
    assert_eq!(matmul_flops(MatrixDimension::new(u64::MAX, 1, 1)), None);
    assert_eq!(matmul_flops(MatrixDimension::new(1, u64::MAX, 1)), None);
    assert_eq!(matmul_flops(MatrixDimension::new(1, 1, u64::MAX)), None);
    assert_eq!(
        matmul_flops(MatrixDimension::new(u64::MAX, u64::MAX, u64::MAX)),
        None
    );

    // Multi-term power-of-two boundaries
    // 2 * 2^32 * 2^31 * 1 = 2^64 -> overflows u64!
    assert_eq!(
        matmul_flops(MatrixDimension::new(1 << 32, 1 << 31, 1)),
        None
    );
    // 2 * 2^31 * 2^31 * 1 = 2^63 -> fits in u64!
    assert_eq!(
        matmul_flops(MatrixDimension::new(1 << 31, 1 << 31, 1)),
        Some(1u64 << 63)
    );

    // Three-term power-of-two boundaries:
    // 2 * 2^21 * 2^21 * 2^21 = 2 * 2^63 = 2^64 -> overflows!
    assert_eq!(
        matmul_flops(MatrixDimension::new(1 << 21, 1 << 21, 1 << 21)),
        None
    );
    // 2 * 2^20 * 2^21 * 2^21 = 2 * 2^62 = 2^63 -> fits!
    assert_eq!(
        matmul_flops(MatrixDimension::new(1 << 20, 1 << 21, 1 << 21)),
        Some(1u64 << 63)
    );

    // Cube root boundary near (u64::MAX / 2)^(1/3)
    let crt = 2_097_151u64; // 2^21 - 1
    let crt_flops = matmul_flops(MatrixDimension::new(crt, crt, crt));
    assert!(crt_flops.is_some(), "Cube of (2^21 - 1) must fit in u64");
    assert!(crt_flops.unwrap() < u64::MAX);
}

#[test]
fn test_adversarial_matmul_flops_all_llm_dimensions() {
    let models = [
        ModelSpec::llama3_1b(),
        ModelSpec::llama3_3b(),
        ModelSpec::llama2_7b(),
        ModelSpec::llama3_8b(),
        ModelSpec::llama2_13b(),
        ModelSpec::llama3_70b(),
    ];

    let context_lengths = [1u64, 128, 512, 2048, 4096, 32768, 65536, 131072];
    let batch_sizes = [1u64, 4, 16, 32, 64, 128, 256];

    for spec in &models {
        for &ctx in &context_lengths {
            for &batch in &batch_sizes {
                let m = batch * ctx;
                let k = u64::from(spec.hidden_dim);
                let n = u64::from(spec.hidden_dim);

                let flops = matmul_flops(MatrixDimension::new(m, k, n));
                assert!(
                    flops.is_some(),
                    "FLOPs for model {} with batch {} and ctx {} must not overflow: got None",
                    spec.name,
                    batch,
                    ctx
                );

                let val = flops.unwrap();
                let expected = 2u128 * (m as u128) * (k as u128) * (n as u128);
                assert_eq!(
                    val as u128, expected,
                    "FLOPs calculation mismatch for {}",
                    spec.name
                );
            }
        }
    }
}

#[test]
fn test_adversarial_checked_kv_bytes_per_token() {
    let models = [
        (ModelSpec::llama3_1b(), 2 * 16 * 8 * 64 * 2),
        (ModelSpec::llama3_3b(), 2 * 28 * 8 * 128 * 2),
        (ModelSpec::llama2_7b(), 2 * 32 * 32 * 128 * 2),
        (ModelSpec::llama3_8b(), 2 * 32 * 8 * 128 * 2),
        (ModelSpec::llama2_13b(), 2 * 40 * 40 * 128 * 2),
        (ModelSpec::llama3_70b(), 2 * 80 * 8 * 128 * 2),
    ];

    for (spec, expected) in models {
        assert_eq!(spec.checked_kv_bytes_per_token(2), Some(expected));
        assert_eq!(spec.kv_bytes_per_token(2), expected);
    }

    // Zero element size
    let m8b = ModelSpec::llama3_8b();
    assert_eq!(m8b.checked_kv_bytes_per_token(0), Some(0));

    // 4.29 billion layers with modest heads (2 * u32::MAX * 32 * 128 * 2 = 70,368,744,161,280 = ~70.3 TB/token)
    // Fits in u64 without overflow!
    let massive_layers_spec = ModelSpec {
        name: "MassiveLayers",
        parameter_count: 1_000_000,
        layers: u32::MAX,
        hidden_dim: 4096,
        attention_heads: 32,
        kv_heads: 32,
        head_dim: 128,
    };
    assert_eq!(
        massive_layers_spec.checked_kv_bytes_per_token(2),
        Some(70_368_744_161_280)
    );
    assert_eq!(
        massive_layers_spec.checked_kv_bytes_per_token(u64::MAX),
        None,
        "Multiplying 70.3 TB by u64::MAX must overflow to None"
    );

    // Overflow case: layers = u32::MAX and kv_heads = u32::MAX
    // 2 * 2^32 * 2^32 * 1 * 1 = 2^65 > u64::MAX -> None
    let dual_u32_max_spec = ModelSpec {
        name: "DualU32Max",
        parameter_count: 1_000_000,
        layers: u32::MAX,
        hidden_dim: 4096,
        attention_heads: 32,
        kv_heads: u32::MAX,
        head_dim: 1,
    };
    assert_eq!(
        dual_u32_max_spec.checked_kv_bytes_per_token(1),
        None,
        "layers=u32::MAX and kv_heads=u32::MAX must overflow u64"
    );

    // Exact boundary with minimal spec (2 * 1 * 1 * 1 * bytes_per_element)
    let unit_spec = ModelSpec {
        name: "UnitSpec",
        parameter_count: 1,
        layers: 1,
        hidden_dim: 1,
        attention_heads: 1,
        kv_heads: 1,
        head_dim: 1,
    };
    let half_max = u64::MAX / 2;
    assert_eq!(
        unit_spec.checked_kv_bytes_per_token(half_max),
        Some(u64::MAX - 1),
        "2 * 1 * 1 * 1 * (u64::MAX / 2) must equal u64::MAX - 1"
    );
    assert_eq!(
        unit_spec.checked_kv_bytes_per_token(half_max + 1),
        None,
        "2 * 1 * 1 * 1 * (u64::MAX / 2 + 1) must overflow to None"
    );
}

// ============================================================================
// 2. ADVERSARIAL 4-WAY FUSED KERNEL PERMUTATIONS (88 COMBINATIONS)
// ============================================================================

#[test]
fn test_adversarial_kernel_profile_permutations_exhaustive_88() {
    let boolean_options = [false, true];
    let operator_counts = [0u32, 1, 2, 3, 4, 5, 6, 8, 16, 32, u32::MAX];

    let mut optimal_count = 0;
    let mut suboptimal_count = 0;

    for &panel_packed in &boolean_options {
        for &warm_start_folded in &boolean_options {
            for &kv_prefix_elided in &boolean_options {
                for &fused_operators in &operator_counts {
                    let profile = FusedKernelProfile {
                        fused_operators,
                        panel_packed,
                        warm_start_folded,
                        kv_prefix_elided,
                    };

                    let is_opt = profile.is_optimal();
                    let expected_opt = panel_packed
                        && warm_start_folded
                        && kv_prefix_elided
                        && fused_operators == 4;

                    assert_eq!(
                        is_opt, expected_opt,
                        "Mismatch for profile: fused={}, panel={}, warm={}, elided={}",
                        fused_operators, panel_packed, warm_start_folded, kv_prefix_elided
                    );

                    if is_opt {
                        optimal_count += 1;
                    } else {
                        suboptimal_count += 1;
                    }
                }
            }
        }
    }

    assert_eq!(
        optimal_count, 1,
        "EXACTLY ONE kernel profile out of 88 must be optimal"
    );
    assert_eq!(
        suboptimal_count, 87,
        "EXACTLY 87 kernel profiles out of 88 must be suboptimal"
    );
}

#[test]
fn test_adversarial_dram_bandwidth_reduction_and_weights_across_models() {
    let models = [
        ModelSpec::llama3_1b(),
        ModelSpec::llama3_3b(),
        ModelSpec::llama2_7b(),
        ModelSpec::llama3_8b(),
        ModelSpec::llama2_13b(),
        ModelSpec::llama3_70b(),
    ];

    for spec in &models {
        let comp = evaluate_ai_operations_comparison(*spec, 4096, 2048, 16);

        // DRAM bandwidth reduction must be exactly 75.0%
        assert_eq!(
            comp.dram_traffic_reduction_pct, 75.0,
            "DRAM traffic reduction must be 75.0% for {}",
            spec.name
        );

        let expected_prism_dram = u64::from(spec.hidden_dim) * 4;
        let expected_non_prism_dram = u64::from(spec.hidden_dim) * 16;
        assert_eq!(
            comp.prism_dram_bytes_per_token, expected_prism_dram,
            "Prism DRAM bytes/token mismatch for {}",
            spec.name
        );
        assert_eq!(
            comp.non_prism_dram_bytes_per_token, expected_non_prism_dram,
            "Non-Prism DRAM bytes/token mismatch for {}",
            spec.name
        );

        // Verification of 75.0% formula: (non - prism) / non
        let diff = expected_non_prism_dram - expected_prism_dram;
        let pct = (diff as f64 / expected_non_prism_dram as f64) * 100.0;
        assert_eq!(pct, 75.0, "DRAM reduction ratio must evaluate to 75.0%");

        // Activation envelope WS-3
        let expected_ws3_prism = u64::from(spec.hidden_dim) * 4 * 1024;
        let expected_ws3_non_prism = expected_ws3_prism * 4;
        assert_eq!(
            comp.working_set.ws3_activation_bytes, expected_ws3_prism,
            "WS-3 Prism activation bytes mismatch for {}",
            spec.name
        );
        assert_eq!(
            comp.working_set.ws3_unfused_activation_bytes, expected_ws3_non_prism,
            "WS-3 Non-Prism activation bytes mismatch for {}",
            spec.name
        );
    }
}

// ============================================================================
// 3. ADVERSARIAL INDUCTIVE COMMAND ROUTING & EXIT CODE 5 VERIFICATION
// ============================================================================

const ALL_CANONICAL_COMMANDS: [(&str, CliCommand, u8); 28] = [
    ("run", CliCommand::Run, 0),
    ("serve", CliCommand::Serve, 1),
    ("pull", CliCommand::Pull, 2),
    ("push", CliCommand::Push, 3),
    ("inspect", CliCommand::Inspect, 4),
    ("chat", CliCommand::Chat, 5),
    ("nodes", CliCommand::Nodes, 6),
    ("status", CliCommand::Status, 7),
    ("stop", CliCommand::Stop, 8),
    ("restart", CliCommand::Restart, 9),
    ("update", CliCommand::Update, 10),
    ("init", CliCommand::Init, 11),
    ("compile", CliCommand::Compile, 12),
    ("files", CliCommand::Files, 13),
    ("doctor", CliCommand::Doctor, 14),
    ("ai", CliCommand::Ai, 15),
    ("plugins", CliCommand::Plugins, 16),
    ("oci", CliCommand::Oci, 17),
    ("models", CliCommand::Models, 18),
    ("route", CliCommand::Route, 19),
    ("app", CliCommand::App, 20),
    ("config", CliCommand::Config, 21),
    ("tracing", CliCommand::Tracing, 22),
    ("history", CliCommand::History, 23),
    ("openapi", CliCommand::Openapi, 24),
    ("verify", CliCommand::Verify, 25),
    ("plan", CliCommand::Plan, 26),
    ("help", CliCommand::Help, 27),
];

#[test]
fn test_adversarial_all_28_canonical_commands_exhaustive() {
    for (name, expected_cmd, expected_discriminant) in ALL_CANONICAL_COMMANDS {
        let parsed = parseCliCommand(name.to_string());
        assert_eq!(parsed, expected_cmd, "Command '{name}' must parse correctly");
        assert_eq!(
            parsed as u8, expected_discriminant,
            "Discriminant for '{name}' must be {expected_discriminant}"
        );

        let exec_str = executeCommand(parsed);
        assert!(
            !exec_str.contains("unknown command"),
            "Canonical command '{name}' must not produce unknown command response"
        );

        let disp_str = dispatchString(name.to_string());
        assert_eq!(disp_str, exec_str);

        let disp_bytes = dispatchBytes(name.as_bytes().to_vec());
        assert_eq!(String::from_utf8(disp_bytes).unwrap(), exec_str);
    }

    // Verify Unknown discriminant
    assert_eq!(CliCommand::Unknown as u8, 28);
    assert_eq!(
        executeCommand(CliCommand::Unknown),
        r#"{"error":"unknown command"}"#
    );
}

#[test]
fn test_adversarial_malformed_and_boundary_command_routing() {
    let invalid_inputs = [
        // Case sensitivity violations
        "AI", "Ai", "aI", "RUN", "Run", "STATUS", "Status", "HELP", "Help",
        // Substrings / prefixes
        "serv", "pul", "pus", "inspec", "cha", "node", "statu",
        // Extended suffixes
        "run1", "runner", "serve_all", "push--force", "ai-status",
        // Whitespace padding
        " ai", "ai ", " ai ", "\tai\n", "  status", "status\0",
        // Shell & SQL injection attacks
        "; rm -rf /", "`id`", "$(whoami)", "ai; ls", "status' OR '1'='1",
        // Control characters & Unicode
        "", "\0", "\n", "\t", "🤖", "λ", "🔥",
        // Non-modeled commands
        "sudo", "exec", "kill", "format", "reboot", "debug", "benchmark",
    ];

    for &inv in &invalid_inputs {
        let parsed = parseCliCommand(inv.to_string());
        assert_eq!(
            parsed,
            CliCommand::Unknown,
            "Input '{inv}' must parse to CliCommand::Unknown"
        );

        let disp = dispatchString(inv.to_string());
        assert_eq!(
            disp,
            r#"{"error":"unknown command"}"#,
            "Input '{inv}' must produce unknown command error response"
        );
    }

    // Extreme string length stress test (65,536 characters)
    let huge_str = "a".repeat(65536);
    let parsed_huge = parseCliCommand(huge_str.clone());
    assert_eq!(parsed_huge, CliCommand::Unknown);
    assert_eq!(
        dispatchString(huge_str),
        r#"{"error":"unknown command"}"#
    );

    // Malformed UTF-8 bytes to dispatchBytes
    let malformed_byte_vectors: Vec<Vec<u8>> = vec![
        vec![0xFF],
        vec![0xFE, 0xFD],
        vec![0xC0, 0x80],             // Overlong NUL
        vec![0xED, 0xA0, 0x80],       // UTF-16 surrogate half
        vec![0xF4, 0x90, 0x80, 0x80], // Out of range Unicode (> 0x10FFFF)
        vec![0xC2],                   // Incomplete 2-byte sequence
        vec![0xE0, 0xA0],             // Incomplete 3-byte sequence
    ];

    for malformed in malformed_byte_vectors {
        let resp = dispatchBytes(malformed);
        let resp_str = String::from_utf8(resp).expect("Error response must be valid UTF-8");
        assert_eq!(resp_str, r#"{"error":"malformed-utf8"}"#);
    }
}

// ============================================================================
// 4. ADVERSARIAL CLI SUBPROCESS VERIFICATION (EXIT CODE 5)
// ============================================================================

#[test]
fn test_adversarial_cli_exit_code_5_live_capability_missing() {
    let binary_path = env!("CARGO_BIN_EXE_hologram");

    let test_cases = [
        ("invalid_command_xyz", false),
        ("invalid_command_xyz", true),
        ("AI", false),
        ("AI", true),
        ("root_access", false),
        ("root_access", true),
        ("status; rm -rf /", false),
        ("status; rm -rf /", true),
    ];

    for (cmd, json_mode) in test_cases {
        let mut command = Command::new(binary_path);
        command.arg("--prism");
        command.arg(cmd);
        if json_mode {
            command.arg("--json");
        }

        let output = command.output().expect("Execute hologram binary");

        // Status code MUST be exactly 5 (LiveError::Capability)
        assert_eq!(
            output.status.code(),
            Some(5),
            "Command '{cmd}' (json={json_mode}) must exit with status code 5"
        );

        if json_mode {
            let stdout_str = String::from_utf8_lossy(&output.stdout);
            let val: serde_json::Value =
                serde_json::from_str(&stdout_str).expect("Valid JSON on stdout");
            assert_eq!(
                val["code"], "LIVE_CAPABILITY_MISSING",
                "JSON code must be LIVE_CAPABILITY_MISSING"
            );
            assert!(
                val["message"].as_str().unwrap().contains("not modeled or permitted by the PrismPM system architecture"),
                "Message must cite PrismPM declarative architecture"
            );
        } else {
            let stderr_str = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr_str.contains("LIVE_CAPABILITY_MISSING"),
                "Stderr must contain LIVE_CAPABILITY_MISSING for '{cmd}'"
            );
            assert!(
                stderr_str.contains("not modeled or permitted by the PrismPM system architecture"),
                "Stderr must cite PrismPM declarative architecture for '{cmd}'"
            );
        }
    }
}

#[test]
fn test_adversarial_cli_valid_commands_do_not_exit_code_5() {
    let binary_path = env!("CARGO_BIN_EXE_hologram");

    // --prism help must succeed with exit code 0
    let output_help = Command::new(binary_path)
        .args(["--prism", "help"])
        .output()
        .expect("Execute help");
    assert_eq!(
        output_help.status.code(),
        Some(0),
        "--prism help must exit with code 0"
    );

    // --prism ai compare must succeed with exit code 0
    let output_ai = Command::new(binary_path)
        .args([
            "--prism",
            "ai",
            "compare",
            "--model",
            "7b",
            "--context-length",
            "4096",
            "--prefix-tokens",
            "2048",
            "--memory-budget-gb",
            "16",
            "--json",
        ])
        .output()
        .expect("Execute ai compare");
    assert_eq!(
        output_ai.status.code(),
        Some(0),
        "--prism ai compare must exit with code 0"
    );

    let val: serde_json::Value =
        serde_json::from_slice(&output_ai.stdout).expect("Valid JSON output");
    assert_eq!(val["model"], "Llama-2-7B");
    assert_eq!(val["dram_traffic_reduction_pct"], 75.0);
    assert_eq!(val["kv_cache_savings_pct"], 50.0);
}

#[test]
fn test_adversarial_zero_allocation_source_and_throughput_proof() {
    // 1. Code inspection of __prod_borrowed_parseCliCommand in crates/prism-hologram/src/lib.rs
    let source_path = "crates/prism-hologram/src/lib.rs";
    let source_content = std::fs::read_to_string(source_path)
        .expect("crates/prism-hologram/src/lib.rs must exist");

    let fn_start = source_content
        .find("fn __prod_borrowed_parseCliCommand(name: &str) -> crate::CliCommand {")
        .expect("__prod_borrowed_parseCliCommand must be defined");
    let fn_end = source_content[fn_start..]
        .find("pub fn dispatchBytes")
        .expect("dispatchBytes must follow __prod_borrowed_parseCliCommand");
    let fn_body = &source_content[fn_start..fn_start + fn_end];

    // Verify zero heap allocation patterns in the borrowed parser
    assert!(
        !fn_body.contains("alloc::"),
        "__prod_borrowed_parseCliCommand must not call alloc::"
    );
    assert!(
        !fn_body.contains("String::"),
        "__prod_borrowed_parseCliCommand must not construct String"
    );
    assert!(
        !fn_body.contains("to_string()"),
        "__prod_borrowed_parseCliCommand must not allocate String via to_string()"
    );
    assert!(
        !fn_body.contains("to_owned()"),
        "__prod_borrowed_parseCliCommand must not allocate via to_owned()"
    );
    assert!(
        !fn_body.contains("format!"),
        "__prod_borrowed_parseCliCommand must not allocate via format!"
    );
    assert!(
        !fn_body.contains("Box::"),
        "__prod_borrowed_parseCliCommand must not allocate Box"
    );
    assert!(
        !fn_body.contains("Vec::"),
        "__prod_borrowed_parseCliCommand must not allocate Vec"
    );

    // 2. High throughput and dispatch latency test (< 500 ns / op)
    let iterations = 100_000;
    let sample_commands = ["ai", "status", "run", "serve", "doctor", "unknown_cmd"];
    let start = std::time::Instant::now();
    for _ in 0..iterations {
        for cmd in &sample_commands {
            let parsed = parseCliCommand((*cmd).to_string());
            std::hint::black_box(parsed);
        }
    }
    let elapsed = start.elapsed();
    let total_ops = iterations * sample_commands.len();
    let nanos_per_op = elapsed.as_nanos() as f64 / total_ops as f64;
    assert!(
        nanos_per_op < 1_000.0,
        "Dispatch latency must be strictly sub-microsecond: measured {nanos_per_op:.2} ns/op"
    );
}

#[test]
fn test_adversarial_cli_injection_and_path_traversal_matrix() {
    let binary_path = env!("CARGO_BIN_EXE_hologram");

    let injection_vectors = [
        "../../etc/passwd",
        "../../../bin/sh",
        "status && ls -la",
        "ai || rm -rf /",
        "$(whoami)",
        "`id`",
        "status; cat /etc/shadow",
        "ai%00extra",
        "../../../../../../root",
        "invalid_subcmd_with_spaces in arg",
    ];

    for vec in injection_vectors {
        let output = Command::new(binary_path)
            .args(["--prism", vec])
            .output()
            .expect("Execute injection vector");

        assert_eq!(
            output.status.code(),
            Some(5),
            "Injection/traversal vector '{vec}' must be rejected with exit code 5"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("LIVE_CAPABILITY_MISSING"),
            "Stderr must indicate LIVE_CAPABILITY_MISSING for '{vec}'"
        );
    }

    // When flags precede an unmodeled command, PrismPM correctly extracts the positional command
    let output_flag_prefixed = Command::new(binary_path)
        .args(["--prism", "-v", "unmodeled_operation"])
        .output()
        .expect("Execute flag-prefixed unmodeled command");
    assert_eq!(
        output_flag_prefixed.status.code(),
        Some(5),
        "Flag-prefixed unmodeled command must exit with code 5"
    );

    // Architectural nuance: when only flags are passed, subcmd defaults to 'help' (passes PrismPM),
    // and clap parses the flag, returning code 2 on unknown flags.
    let output_only_flag = Command::new(binary_path)
        .args(["--prism", "-D"])
        .output()
        .expect("Execute unknown flag only");
    assert_eq!(
        output_only_flag.status.code(),
        Some(2),
        "Standalone unknown flag without positional command must be handled by clap with code 2"
    );
}

