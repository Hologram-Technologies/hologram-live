# `.holo` Library Archives Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give `.holo` a stated library-archive contract — an explicit `library` marker enforced at compile time, and a typed `library_archive` plan blocker so the plan report stops reporting a library as runnable.

**Architecture:** A library archive is one whose canonical `AppManifest.primary` is `None` — a shape upstream already permits and encodes with a `NO_PRIMARY` sentinel. This plan adds a source-manifest `library` boolean enforced as `library == primary.is_none()`, and one new `PlanBlocker` variant pushed during planning. Because every execution path funnels through `into_application_plan()`, that single blocker makes `run`, `holo load`, and sessions all refuse a library with one typed error. No canonical manifest, application directory, wire, or physical format change.

**Tech Stack:** Rust 1.94, upstream `uor-hologram` (pinned rev `2bda6a9`) for `archive` and `space`, serde, cucumber for BDD, `just` for verification tasks.

**Spec:** `docs/superpowers/specs/2026-09-03-holo-library-archives-design.md`

## Global Constraints

- Physical `.holo` version 4 only. This plan introduces no new physical version, no new archive extension, and no migration (ADR 016).
- Application-directory schema stays at version 2. Do not add a `library` field to `HoloDirectory` — it would be redundant with `primary_layer: null` and violates ADR 006's projection rule.
- Source-manifest schema stays at version 4. `CompileManifest` is `#[serde(deny_unknown_fields)]`; the new field must be declared there before any manifest can carry it.
- Do not add a `LiveError` variant. The library blocker maps to the existing `LIVE_CAPABILITY_MISSING`; precision is carried by the blocker's `kind()` string `"library_archive"`.
- The library blocker applies to the **root** application only. Child applications with no primary remain legal — that is the library consumption path.
- Canonical `AppManifest` is not modified. `primary: None` remains the single canonical signal.
- Every commit must leave `just check`, `just clippy`, and `just test` passing.
- `just test` runs single-threaded (`--test-threads=1`) because several tests own real subprocesses.

---

### Task 1: `library_archive` plan blocker

Makes the plan report tell the truth about library archives, and removes the stale dead code that was supposed to do this.

**Files:**
- Modify: `src/application_plan.rs` (add blocker variant; push it in `explain_application`; delete `require_single_primary` and `ApplicationPlan::primary`; add three tests)
- Modify: `src/holo_provider.rs:505-510` (retype the now-unreachable primary check)
- Modify: `src/holo.rs` (update two existing tests whose asserted error message changes)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces: `PlanBlocker::LibraryArchive` (unit variant) with `kind() == "library_archive"` and `error_code() == "LIVE_CAPABILITY_MISSING"`. Task 3 asserts on that `kind` string through the JSON plan output.

**Note on blocker ordering.** The blocker is appended inside `explain_application`, which runs before `registry.evaluate(&mut report)` in both production callers, so it always precedes provider-availability blockers. For a well-formed library archive no resolution blockers exist, so it lands at index 0 and is the error `into_application_plan()` returns. For a *malformed* library (for example a thin archive with unresolvable content) a resolution blocker precedes it, which is correct — that must be fixed regardless.

- [ ] **Step 1: Write the failing test**

Add to the `mod tests` block in `src/application_plan.rs`, next to the other blocker tests:

```rust
    #[test]
    fn a_library_root_is_not_runnable() {
        let capabilities = test_capabilities();
        let capabilities_kappa = address_bytes(capabilities);
        let wasm = b"library wasm";
        let wasm_kappa = address_bytes(wasm);
        let manifest = AppManifest {
            primary: None,
            requires: capabilities_kappa,
            layers: vec![wasm_layer(wasm_kappa, "holo_run")],
            children: Vec::new(),
        };
        let bytes = write_archive(
            &manifest,
            &[(&capabilities_kappa, capabilities), (&wasm_kappa, wasm)],
        );

        let mut report =
            explain_application(&bytes, PlanLimits::default(), |_| Ok(None)).expect("plan");
        report.evaluate_providers(available);

        assert!(!report.runnable());
        assert_eq!(report.blockers.len(), 1);
        assert_eq!(report.blockers[0].kind(), "library_archive");
        assert_eq!(report.blockers[0].error_code(), "LIVE_CAPABILITY_MISSING");

        let error = report
            .into_application_plan()
            .expect_err("a library archive cannot become an execution plan");
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");
        assert!(error.to_string().contains("library"), "{error}");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --package hologram-live --lib application_plan::tests::a_library_root_is_not_runnable -- --exact --nocapture`

Expected: FAIL. The provider is available and no blocker is produced, so `report.runnable()` is `true` and the first assertion fails. This is the bug being fixed.

- [ ] **Step 3: Add the blocker variant**

In `src/application_plan.rs`, add the variant at the end of the `PlanBlocker` enum, after `ExecutionShapeUnsupported`:

```rust
    /// The root manifest declares no primary layer: the archive is a library,
    /// not an executable application.
    LibraryArchive,
```

Add to `kind()`:

```rust
            Self::LibraryArchive => "library_archive",
```

Extend the existing `error_code()` arm so it reads:

```rust
            Self::ProviderUnavailable { .. }
            | Self::ExecutionShapeUnsupported { .. }
            | Self::LibraryArchive => "LIVE_CAPABILITY_MISSING",
```

Add to `message()`:

```rust
            Self::LibraryArchive => format!(
                "application {application_kappa} is a library archive: it declares no primary layer and cannot be executed"
            ),
```

- [ ] **Step 4: Push the blocker during planning**

In `src/application_plan.rs`, in `explain_application`, immediately before the closing `Ok(ApplicationPlanReport {` expression, insert:

```rust
    let mut blockers = std::mem::take(&mut closure.blockers);
    if manifest.primary.is_none() {
        blockers.push(PlanBlocker::LibraryArchive);
    }
```

Then change the struct field initializer from `blockers: closure.blockers,` to `blockers,`.

`closure` is already bound as `let mut closure = ClosureResolver { ... }`, so `std::mem::take` compiles without further changes. `manifest` here is the decoded **root** manifest — do not apply this to child manifests.

- [ ] **Step 5: Delete the stale dead code**

In `src/application_plan.rs`, delete the entire `require_single_primary` method. It has no callers, and its second branch ("multi-layer lifecycle is not connected yet") is stale — multi-layer applications work today, so wiring it would break them.

In the same file, delete the `primary` method from `impl ApplicationPlan`:

```rust
    pub fn primary(&self) -> Option<&ResolvedLayer> {
        self.primary_layer
            .and_then(|position| self.layers.get(position as usize))
    }
```

It also has no callers.

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test --package hologram-live --lib application_plan::tests::a_library_root_is_not_runnable -- --exact --nocapture`

Expected: PASS.

- [ ] **Step 7: Add the root-only regression test**

This guards the rule that a library composed as a child must NOT block its parent. Add to `mod tests` in `src/application_plan.rs`:

```rust
    #[test]
    fn a_library_child_does_not_block_its_parent() {
        let capabilities = test_capabilities();
        let capabilities_kappa = address_bytes(capabilities);
        let root_layer = b"root wasm";
        let root_layer_kappa = address_bytes(root_layer);
        let child_layer = b"child library wasm";
        let child_layer_kappa = address_bytes(child_layer);

        let child_manifest = AppManifest {
            primary: None,
            requires: capabilities_kappa,
            layers: vec![wasm_layer(child_layer_kappa, "child")],
            children: Vec::new(),
        };
        let child_manifest_bytes = child_manifest.canonicalize();
        let child_kappa = address_bytes(&child_manifest_bytes);

        let manifest = AppManifest {
            primary: Some(0),
            requires: capabilities_kappa,
            layers: vec![wasm_layer(root_layer_kappa, "root")],
            children: vec![(child_kappa, capabilities_kappa)],
        };
        let bytes = write_archive(
            &manifest,
            &[
                (&capabilities_kappa, capabilities),
                (&root_layer_kappa, root_layer),
                (&child_layer_kappa, child_layer),
                (&child_kappa, child_manifest_bytes.as_slice()),
            ],
        );

        let mut report =
            explain_application(&bytes, PlanLimits::default(), |_| Ok(None)).expect("plan");
        report.evaluate_providers(available);

        assert!(
            !report
                .blockers
                .iter()
                .any(|blocker| blocker.kind() == "library_archive"),
            "a child with no primary must not block its parent: {:?}",
            report.blockers.iter().map(PlanBlocker::kind).collect::<Vec<_>>()
        );
        assert!(report.runnable());
        report
            .into_application_plan()
            .expect("a parent composing a library child still plans");
    }
```

- [ ] **Step 8: Add the multi-layer regression test**

This guards the `require_single_primary` deletion — proving nothing reintroduces its stale multi-layer restriction. Add to `mod tests` in `src/application_plan.rs`:

```rust
    #[test]
    fn multi_layer_applications_remain_runnable() {
        let capabilities = test_capabilities();
        let capabilities_kappa = address_bytes(capabilities);
        let first = b"first layer";
        let first_kappa = address_bytes(first);
        let second = b"second layer";
        let second_kappa = address_bytes(second);
        let manifest = AppManifest {
            primary: Some(1),
            requires: capabilities_kappa,
            layers: vec![
                wasm_layer(first_kappa, "first"),
                wasm_layer(second_kappa, "second"),
            ],
            children: Vec::new(),
        };
        let bytes = write_archive(
            &manifest,
            &[
                (&capabilities_kappa, capabilities),
                (&first_kappa, first),
                (&second_kappa, second),
            ],
        );

        let mut report =
            explain_application(&bytes, PlanLimits::default(), |_| Ok(None)).expect("plan");
        report.evaluate_providers(available);

        assert!(report.blockers.is_empty());
        assert!(report.runnable());
    }
```

- [ ] **Step 9: Run both regression tests**

Run: `cargo test --package hologram-live --lib application_plan::tests:: -- --nocapture`

Expected: PASS, including `a_library_child_does_not_block_its_parent` and `multi_layer_applications_remain_runnable`.

- [ ] **Step 10: Retype the now-unreachable runtime check**

In `src/holo_provider.rs`, in `prepare_and_start_with_admitted_grants`, change:

```rust
    let primary_layer = plan.primary_layer.ok_or_else(|| {
        LiveError::Capability(format!(
            "application {} has no primary exit-bearing layer",
            plan.identity.application_kappa
        ))
    })?;
```

to:

```rust
    let primary_layer = plan.primary_layer.ok_or_else(|| {
        LiveError::Conflict(format!(
            "runtime received application {} with no primary layer; the planner must reject library archives before preparation",
            plan.identity.application_kappa
        ))
    })?;
```

The blocker from Step 4 makes this unreachable — an `ApplicationPlan` with `primary_layer: None` can no longer be constructed. Keep it as defense in depth, but as an internal-invariant error matching the file's existing convention (see `application_requested_capabilities`, which uses `LiveError::Conflict` for "runtime lost ..."). Leave `application_primary_layer` unchanged; it serves child View applications.

- [ ] **Step 11: Update the two existing tests whose error message changes**

Both are in `mod tests` in `src/holo.rs`. They build `AppManifest` directly and assert on an execute/load error that this task deliberately replaces, because `library_archive` now precedes the provider blocker.

In `load_rejects_a_view_only_archive`, replace the two assertions after `expect_err("must fail")` with:

```rust
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");
        assert!(error.to_string().contains("library archive"), "{error}");
```

A view-only archive is a library: a View layer is non-exit-bearing and can never be a primary, so this archive could never have run.

In `model_only_execution_reports_the_missing_inference_provider`, leave the plan-level assertions untouched — they use `.any(...)` and the `provider_unavailable` blocker is still produced and still reported. Replace only the three assertions after `expect_err("model provider is not connected")` with:

```rust
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");
        assert!(error.to_string().contains("library archive"), "{error}");
```

This is the intended improvement: "the inference provider is not connected" was misleading, because installing that provider would not have made the archive runnable.

- [ ] **Step 12: Run the full library test suite**

Run: `cargo test --package hologram-live --lib -- --test-threads=1`

Expected: PASS. If `load_rejects_a_view_only_archive` or `model_only_execution_reports_the_missing_inference_provider` still fail, re-read Step 11 — their assertions must target the library message, not the provider message.

- [ ] **Step 13: Verify formatting and lints**

Run: `just fmt && just clippy`

Expected: both succeed. Clippy runs with `-D warnings`; if it reports the new enum variant as needing documentation or a match arm as redundant, fix it before committing.

- [ ] **Step 14: Commit**

```bash
git add src/application_plan.rs src/holo_provider.rs src/holo.rs
git commit -m "fix(holo): report library archives as non-runnable

Add a typed library_archive plan blocker for archives whose root manifest
declares no primary layer. Previously ApplicationPlanReport::runnable()
never consulted primary_layer, so a no-primary archive with available
providers reported runnable: true and failed only at start.

The blocker applies to the root only; child applications with no primary
remain legal, which is the library composition path.

Delete require_single_primary and ApplicationPlan::primary, both of which
had no callers. require_single_primary also carried a stale multi-layer
restriction that would have broken working archives.

Retype the now-unreachable primary check in prepare_and_start as an
internal invariant error."
```

---

### Task 2: `library` source-manifest marker

Catches the failure inference cannot: a manifest that omits `primary` by mistake.

**Files:**
- Modify: `src/compile.rs` (add `library` field; enforce the biconditional in `validate_compile_manifest`; add `is_false` helper; add three tests; update one existing test)
- Modify: `src/cli/app.rs:222-228` (the `CompileManifest` struct literal gains the new field)
- Modify: `features/fixtures/view-app/hologram.json` (declare the marker)

**Interfaces:**
- Consumes: nothing from Task 1. These tasks are independent and can be reviewed separately.
- Produces: `CompileManifest.library: bool`. Task 3 writes a fixture manifest carrying `"library": true`, which cannot parse until this field exists.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/compile.rs`, following the existing tempdir-and-write pattern used by `wasm_layers_require_explicit_entry_and_contract`:

```rust
    #[test]
    fn a_library_manifest_compiles_without_a_primary() {
        let directory = tempfile::tempdir().expect("tempdir");
        std::fs::write(directory.path().join("app.wasm"), b"wasm bytes").expect("wasm");
        let manifest_path = directory.path().join("hologram.json");
        std::fs::write(
            &manifest_path,
            r#"{
                "schema_version": 4,
                "library": true,
                "layers": [{
                    "kind": "wasm",
                    "path": "app.wasm",
                    "entry": "holo_run",
                    "contract": "hologram:guest/core-wasm@1"
                }]
            }"#,
        )
        .expect("manifest");

        let compiled = compile_manifest(&manifest_path).expect("compile library archive");
        let inspection =
            inspect_bytes("library", "library.holo", &compiled.bytes).expect("inspect");
        let directory = inspection.directory.expect("application directory");
        assert_eq!(directory.primary_layer, None);
    }

    #[test]
    fn a_library_manifest_rejects_a_declared_primary() {
        let directory = tempfile::tempdir().expect("tempdir");
        std::fs::write(directory.path().join("app.wasm"), b"wasm bytes").expect("wasm");
        let manifest_path = directory.path().join("hologram.json");
        std::fs::write(
            &manifest_path,
            r#"{
                "schema_version": 4,
                "library": true,
                "primary": 0,
                "layers": [{
                    "kind": "wasm",
                    "path": "app.wasm",
                    "entry": "holo_run",
                    "contract": "hologram:guest/core-wasm@1"
                }]
            }"#,
        )
        .expect("manifest");

        let error = compile_manifest(&manifest_path).expect_err("must reject");
        assert_eq!(error.code(), "LIVE_CONFIG_INVALID");
        assert!(error.to_string().contains("library"), "{error}");
    }

    #[test]
    fn an_absent_primary_requires_the_library_marker() {
        let directory = tempfile::tempdir().expect("tempdir");
        std::fs::write(directory.path().join("app.wasm"), b"wasm bytes").expect("wasm");
        let manifest_path = directory.path().join("hologram.json");
        std::fs::write(
            &manifest_path,
            r#"{
                "schema_version": 4,
                "layers": [{
                    "kind": "wasm",
                    "path": "app.wasm",
                    "entry": "holo_run",
                    "contract": "hologram:guest/core-wasm@1"
                }]
            }"#,
        )
        .expect("manifest");

        let error = compile_manifest(&manifest_path).expect_err("must reject");
        assert_eq!(error.code(), "LIVE_CONFIG_INVALID");
        assert!(error.to_string().contains("library"), "{error}");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --package hologram-live --lib compile::tests::a_library_manifest compile::tests::an_absent_primary -- --nocapture`

Expected: FAIL. The first two fail at parse — `CompileManifest` is `deny_unknown_fields`, so `"library"` is rejected as an unknown field. The third fails because a manifest without `primary` currently compiles successfully.

- [ ] **Step 3: Add the field and the serde helper**

In `src/compile.rs`, add the field to `CompileManifest` immediately after `primary`:

```rust
    #[serde(default, skip_serializing_if = "is_false")]
    pub library: bool,
```

Add this free function near the top of the file, after the `CURRENT_MANIFEST_SCHEMA_VERSION` constant:

```rust
/// `skip_serializing_if` predicate: keep generated manifests free of
/// `"library": false`, which is the default for every executable application.
fn is_false(value: &bool) -> bool {
    !*value
}
```

- [ ] **Step 4: Enforce the biconditional**

In `src/compile.rs`, in `validate_compile_manifest`, immediately after the existing `schema_version` check and before the layer loop, insert:

```rust
    if specification.library != specification.primary.is_none() {
        return Err(LiveError::Config(
            if specification.library {
                "a library manifest declares no primary layer; remove \"primary\" or remove \"library\""
            } else {
                "a manifest without a primary layer must declare \"library\": true"
            }
            .to_owned(),
        ));
    }
```

`compile_manifest_with_options` already calls `validate_compile_manifest` before building any layer content, so both errors are raised before work begins. No second check site is needed.

- [ ] **Step 5: Fix the generator struct literal**

`src/cli/app.rs` builds a `CompileManifest` literal, which will no longer compile. Add the field so the generated manifest always satisfies the biconditional:

```rust
    let specification = CompileManifest {
        schema_version: 4,
        library: primary.is_none(),
        primary,
        requires,
        layers: std::mem::take(&mut layers),
        children,
    };
```

The `validate_compile_manifest(&specification)?` call on the next line then always passes for generated manifests.

- [ ] **Step 6: Update the existing no-primary test and the fixture**

In `src/compile.rs`, the inference-model test that asserts `directory.primary_layer == None` writes a manifest with no `primary`. Add the marker to its embedded JSON so it still compiles:

```rust
            r#"{
                "schema_version": 4,
                "library": true,
                "layers": [{
                    "kind": "inference-model",
                    "path": "model.bundle",
                    "entry": "ai.default",
                    "engine": "uor-r4"
                }]
            }"#,
```

Then update `features/fixtures/view-app/hologram.json`, the only in-repo manifest without a `primary`:

```json
{
  "schema_version": 4,
  "library": true,
  "layers": [
    {
      "kind": "view",
      "path": "ui",
      "surface": "portable"
    }
  ]
}
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test --package hologram-live --lib -- --test-threads=1`

Expected: PASS, including the three new tests and the updated inference-model test.

- [ ] **Step 8: Verify formatting and lints**

Run: `just fmt && just clippy`

Expected: both succeed.

- [ ] **Step 9: Commit**

```bash
git add src/compile.rs src/cli/app.rs features/fixtures/view-app/hologram.json
git commit -m "feat(compile): require an explicit library marker

Add \"library\" to the source manifest, enforced as
library == primary.is_none() in validate_compile_manifest. A manifest that
omits primary by mistake now fails at compile with LIVE_CONFIG_INVALID
instead of producing an archive that dies at start.

The marker is a compile-time assertion only. Canonical AppManifest and the
application directory are unchanged; primary: None remains the single
canonical signal, so no schema version moves."
```

---

### Task 3: BDD scenario for the library boundary

`features/README.md` requires a scenario alongside a new public boundary. Compiling and planning a library archive is one.

**Files:**
- Create: `features/fixtures/wasm-library/hologram.json`
- Create: `features/fixtures/wasm-library/transform.wat` (copied from the existing wasm fixture)
- Modify: `features/suites/s2_holo_exec/run.feature` (add one scenario)
- Modify: `tests/bdd.rs` (add one `given`, one `when`, two `then` steps)

**Interfaces:**
- Consumes: `CompileManifest.library` from Task 2 (the fixture manifest cannot parse without it) and `PlanBlocker::LibraryArchive` from Task 1 (the assertion targets `kind == "library_archive"`). **Both Task 1 and Task 2 must be merged before this task runs.**
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Create the fixture**

```bash
mkdir -p features/fixtures/wasm-library
cp features/fixtures/wasm-app/transform.wat features/fixtures/wasm-library/transform.wat
cat > features/fixtures/wasm-library/hologram.json <<'JSON'
{
  "schema_version": 4,
  "library": true,
  "layers": [
    {
      "kind": "wasm",
      "path": "transform.wat",
      "entry": "holo_run",
      "contract": "hologram:guest/core-wasm@1"
    }
  ]
}
JSON
```

The layer keeps its explicit `entry` and `contract`: ADR 016 requires both for every Wasm layer, and only `primary` is absent. This is the archive shape that exposed the original bug — a wasm layer whose provider is available, so nothing else makes the plan non-runnable.

- [ ] **Step 2: Write the failing scenario**

Append to `features/suites/s2_holo_exec/run.feature`:

```gherkin
  Scenario: a library archive plans but refuses to run
    Given the example wasm library manifest
    And a fresh Hologram home
    When I compile the application
    Then the compile command succeeds
    When I plan the compiled archive directly
    Then the direct plan reports the archive is a library
    When I run the compiled library archive directly
    Then the run fails with a library-archive error
```

- [ ] **Step 3: Add the step definitions**

In `tests/bdd.rs`, add next to the existing `example_manifest` and `wasm_manifest` givens:

```rust
#[given("the example wasm library manifest")]
fn wasm_library_manifest(world: &mut BddWorld) {
    world.manifest = Some(
        workspace_root()
            .join("features")
            .join("fixtures")
            .join("wasm-library")
            .join("hologram.json"),
    );
    world.temporary = Some(tempfile::tempdir().expect("create scenario directory"));
}
```

Add next to `direct_plan_reports_unavailable_view_surface`:

```rust
#[then("the direct plan reports the archive is a library")]
fn direct_plan_reports_library_archive(world: &mut BddWorld) {
    let plan = world.plan_result.as_ref().expect("plan result");
    assert_eq!(plan["execution_target"], "direct");
    assert_eq!(plan["runnable"], false);
    assert!(plan["primary_layer"].is_null());
    assert_eq!(plan["layers"][0]["provider"]["status"], "available");
    let blockers = plan["blockers"].as_array().expect("blockers");
    assert_eq!(blockers.len(), 1);
    assert_eq!(blockers[0]["kind"], "library_archive");
    assert_eq!(blockers[0]["error_code"], "LIVE_CAPABILITY_MISSING");
}
```

Add next to `run_local_archive_without_grant`, which is the pattern for a run that is expected to fail:

```rust
#[when("I run the compiled library archive directly")]
fn run_local_library_archive(world: &mut BddWorld) {
    world.command_output = Some(
        Command::new(env!("CARGO_BIN_EXE_hologram"))
            .arg("--json")
            .arg("run")
            .arg(world.output_path.as_ref().expect("compiled archive"))
            .env("HOME", home_path(world))
            .output()
            .expect("run local library archive"),
    );
}

#[then("the run fails with a library-archive error")]
fn run_fails_library_archive(world: &mut BddWorld) {
    let output = world.command_output.as_ref().expect("run output");
    assert!(!output.status.success(), "a library archive must not run");
    let error: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse JSON error");
    assert_eq!(error["code"], "LIVE_CAPABILITY_MISSING");
    assert!(
        error["message"]
            .as_str()
            .expect("error message")
            .contains("library archive"),
        "{error}"
    );
}
```

- [ ] **Step 4: Run the scenario**

Run: `just bdd`

Expected: PASS, including the new scenario. The pre-existing view scenario in `features/suites/s0_cli/compile.feature` must also still pass — its blocker assertion uses `.any(...)`, so the added `library_archive` blocker does not break it. If it fails, confirm Task 2's fixture update to `features/fixtures/view-app/hologram.json` was applied.

- [ ] **Step 5: Run the full verification suite**

Run: `just fmt && just check && just clippy && just test && just bdd`

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add features/fixtures/wasm-library features/suites/s2_holo_exec/run.feature tests/bdd.rs
git commit -m "test(bdd): cover the library archive boundary

Add an enforced scenario compiling a wasm library archive, planning it,
and confirming the plan is non-runnable with a single library_archive
blocker before the run refuses with a typed error."
```

---

## Verification

After all three tasks:

```bash
just fmt
just check
just clippy
just test
just bdd
```

## Out of scope

Recorded in the spec's "Adjacent work" section, deliberately excluded here:

- **D — remote content resolution.** The next design pass. `ResolutionSource::ConfiguredResolver` is the reserved seam; wiring it requires making the `resolve_local` closure async.
- **B — linkable cross-archive calls.** Blocked on an upstream `AppManifest` change.
- **C — host-side SDK crate.** Requires decoupling `utoipa` from the format layer and the Python builders from `holo_provider`.
