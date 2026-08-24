# Hologram Live BDD suites

Gherkin scenarios under `features/suites` describe behavior at public product
boundaries. The Cucumber runner in `tests/bdd.rs` drives the real `hologram`
binary rather than calling command handlers directly.

Scenarios tagged `@status:enforced` must execute. Add a scenario alongside a
new CLI, desktop, HTTP, or gRPC interface, and keep test-only setup in the
runner world or `features/fixtures`.

Run the suite with:

```sh
just bdd
```
