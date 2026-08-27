# HoloTrade quote engine as a `.holo` application

A spike that answers: can [HoloTrade](https://github.com/wilcompute/Holotrade)
(MIT) run as a Hologram application? The browser UI cannot — `.holo` archives
package executable layers, not websites — but HoloTrade's deterministic
pricing engine is exactly the bytes-in/bytes-out shape a Wasm layer wants.

This example ports the single-node quote pipeline to the `core-wasm-v1`
guest contract. Given one self-contained JSON request it returns the full
price decomposition the HoloTrade exchange UI shows:

```
P = P_base × E × G × D × H × Q × L     (clamped to the operating floor)
```

with every multiplier, the floor components (energy + maintenance reserve +
capital recovery), margin, carbon, and the locality cost computed from the
W(3,3) symplectic geometry (the 40 projective points are rebuilt in the
guest; adjacency is the symplectic form over F₃⁴).

Ported from HoloTrade (MIT), commit `main` as of 2026-08:

| HoloTrade source | What was ported |
| --- | --- |
| `js/pricing.js` | `quote()`, the six multipliers, the floor |
| `js/energy.js` | `multiplier()`, `hourlyEnergyCost()`, `hourlyCarbon()` |
| `js/fleet.js` | `specialisationScore()`, `fitness()`, `maintenanceReserve()`, `capitalRecovery()`, `CAPEX_MULTIPLE` |
| `js/substrate.js` | `buildPoints()`, `symplecticForm()`, `route()`, `fabricDistance()`, `migrationCost()`, `magicMultiplier()` |

Out of scope (deliberately): the live energy tick loop, fleet evolution,
demand response, order-book depth/impact, execution receipts — all stateful
or stochastic surfaces that would need a different input contract. The
guest is a pure function of its request.

## Build and run

No Cargo project and no crates: the guest is one file with zero
dependencies, so plain `rustc` is enough.

```bash
rustup target add wasm32-unknown-unknown
cd examples/holotrade-quote
rustc --target wasm32-unknown-unknown -O -C panic=abort \
  --crate-type cdylib guest.rs -o app.wasm

hologram compile hologram.json -o holotrade-quote.holo
hologram run holotrade-quote.holo --input request.json --output-format text
```

`app.wasm` is a build artifact and is not committed.

## Request schema

```json
{
  "node": {
    "id": "gpu-a100-07",
    "addr": [3, 17],
    "utilisation": 0.85, "utilisationEMA": 0.8,
    "jobsCompleted": 4200, "jobsFailed": 130,
    "specialisation": { "llm-train": 0.84 },
    "genome": { "throughput": 0.78, "convergenceRate": 0.71, "faultResilience": 0.66 },
    "lineage": { "generation": 4 },
    "health": { "derate": 0.96, "hazard": 0.02, "correctableErrors": 1500, "wear": 0.18 },
    "hardware": { "kind": "gpu", "baseRate": 31.4, "tdp": 700,
                  "lifeHours": 43800, "magicCapable": false, "thermalSensitivity": 1.0 }
  },
  "workloads": [
    { "id": "llm-train", "magicBudget": 0, "geneEmphasis": ["throughput", "convergenceRate"] }
  ],
  "workloadId": "llm-train",
  "anchorAddress": [3, 22],
  "energy": { "baseEnergy": 45.0, "price": 62.0, "carbon": 380.0, "pue": 1.35 },
  "balancerEnabled": true
}
```

`energy.price` / `energy.carbon` are the *current* grid $/MWh and kg
CO₂e/MWh — the live tick loop stays outside; the caller supplies one
observation. `anchorAddress: null` prices no locality term. A workload
with `magicBudget > 0` against `magicCapable: false` hardware returns
`serviceable: false` with a null price, mirroring the JS engine.

## Fidelity

The port was differentially checked against HoloTrade's own JS engine
(`PricingEngine.quote` over stub fleet/energy objects) on 10 scenarios —
hot/cold/in-band nodes, balancer off, quantum on classical silicon
(unserviceable), quantum on photonic, null/same/far anchors, fresh node
with no history, and an energy price spike hitting the clamp. All numeric
fields agree to 1e-9 relative tolerance (JS and Rust `powf`/`ln`/`log10`
differ in final ULPs).
