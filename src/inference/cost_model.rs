//! UOR / Prism formal inference cost model.
//!
//! Declarative implementation mirroring the formal Lean specification in
//! `Hologram.Inference`. Provides exact arithmetic for matmul FLOP bounds,
//! KV-cache token elision, and optimal fused-kernel verification.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatrixDimension {
    pub m: u64,
    pub k: u64,
    pub n: u64,
}

impl MatrixDimension {
    pub const fn new(m: u64, k: u64, n: u64) -> Self {
        Self { m, k, n }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FusedKernelProfile {
    pub fused_operators: u32,
    pub panel_packed: bool,
    pub warm_start_folded: bool,
    pub kv_prefix_elided: bool,
}

impl FusedKernelProfile {
    /// Optimal profile: 4 fused operators (FU-1..FU-4), panel-packed tensors,
    /// folded warm-start, and prefix-elided KV-cache.
    pub const fn optimal() -> Self {
        Self {
            fused_operators: 4,
            panel_packed: true,
            warm_start_folded: true,
            kv_prefix_elided: true,
        }
    }

    /// Verifies if a kernel profile satisfies global optimality criteria
    /// specified in `Hologram.Inference.isOptimalFusedKernel`.
    pub const fn is_optimal(&self) -> bool {
        self.panel_packed
            && self.warm_start_folded
            && self.kv_prefix_elided
            && self.fused_operators == 4
    }
}

/// Compute formal matrix multiplication FLOPs: 2 * M * K * N with checked overflow.
pub fn matmul_flops(dim: MatrixDimension) -> Option<u64> {
    2u64.checked_mul(dim.m)?
        .checked_mul(dim.k)?
        .checked_mul(dim.n)
}

/// Compute effective KV-cache token count after prefix elision.
pub fn kv_effective_tokens(total_tokens: u64, prefix_tokens: u64) -> u64 {
    total_tokens.saturating_sub(prefix_tokens)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceCostProfile {
    pub service: &'static str,
    pub cost_model: &'static str,
    pub status: &'static str,
    pub dimension: MatrixDimension,
    pub matmul_flops: Option<u64>,
    pub effective_tokens: u64,
    pub kernel_profile: FusedKernelProfile,
    pub is_optimal: bool,
}

impl InferenceCostProfile {
    pub fn evaluate(dim: MatrixDimension, total_tokens: u64, prefix_tokens: u64) -> Self {
        let flops = matmul_flops(dim);
        let eff_tokens = kv_effective_tokens(total_tokens, prefix_tokens);
        let kernel = FusedKernelProfile::optimal();
        let is_optimal = kernel.is_optimal();

        Self {
            service: "hologram-ai",
            cost_model: "uor-prism",
            status: if is_optimal { "optimal" } else { "suboptimal" },
            dimension: dim,
            matmul_flops: flops,
            effective_tokens: eff_tokens,
            kernel_profile: kernel,
            is_optimal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonical_matmul_flops() {
        let dim = MatrixDimension::new(1, 4, 4);
        assert_eq!(matmul_flops(dim), Some(32));
    }

    #[test]
    fn test_canonical_kv_effective_tokens() {
        assert_eq!(kv_effective_tokens(100, 80), 20);
        assert_eq!(kv_effective_tokens(50, 100), 0);
    }

    #[test]
    fn test_optimal_fused_kernel() {
        let optimal = FusedKernelProfile::optimal();
        assert!(optimal.is_optimal());

        let suboptimal = FusedKernelProfile {
            fused_operators: 3,
            panel_packed: true,
            warm_start_folded: true,
            kv_prefix_elided: true,
        };
        assert!(!suboptimal.is_optimal());
    }

    #[test]
    fn test_inference_cost_profile_evaluation() {
        let profile = InferenceCostProfile::evaluate(MatrixDimension::new(1, 4, 4), 100, 80);
        assert_eq!(profile.service, "hologram-ai");
        assert_eq!(profile.cost_model, "uor-prism");
        assert_eq!(profile.status, "optimal");
        assert_eq!(profile.matmul_flops, Some(32));
        assert_eq!(profile.effective_tokens, 20);
        assert!(profile.is_optimal);
    }
}
