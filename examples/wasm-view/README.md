# Wasm + View `.holo` application

This example composes an exit-bearing Wasm primary with a portable View layer.
The frontend sends `application.invoke` intent v1 messages to the primary; the
primary returns an ASCII-uppercase transformation. The View has no ambient
Tauri, shell, filesystem, network, or localhost authority.

Compile it as one self-contained archive:

```bash
hologram compile examples/wasm-view/hologram.json \
  --output target/wasm-view.holo
```

Import `target/wasm-view.holo` from Hologram Desktop and run it from the
Applications screen to exercise the current one-shot attachment and
root-completion lifecycle. Direct CLI execution deliberately reports that the
portable View surface is unavailable because a headless process cannot attach
the UI.

The Desktop adapter test compiles this exact manifest, attaches its bundle
through a display-independent window host, submits one real intent to the Wasm
primary, preserves root-primary completion, and observes reverse shutdown.
Keeping the form open for user-driven turns requires the next explicit
application-session API; the current one-shot contract closes its View when the
root primary completes.
